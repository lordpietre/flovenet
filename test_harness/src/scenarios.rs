use std::time::Duration;

use libp2p::gossipsub;
use libp2p::swarm::SwarmEvent;
use tracing::info;

use test_harness::{
    wait_for_event, Scenario, ScenarioResult, TestCheck, TestNode, TestOrchestrator,
};

// ── Scenario 1: P2P Mesh Formation ─────────────────────────

pub struct P2pMeshScenario {
    pub node_count: usize,
}

#[async_trait::async_trait]
impl Scenario for P2pMeshScenario {
    fn name(&self) -> &str {
        "p2p_mesh"
    }

    async fn run(&self, orch: &TestOrchestrator) -> ScenarioResult {
        let start = std::time::Instant::now();
        let mut checks = Vec::new();

        info!("Creating {count} P2P nodes...", count = self.node_count);
        for i in 0..self.node_count {
            let port = 21000 + i as u16;
            let node = TestNode::new(port);
            info!("  Node {i}: {} on port {port}", node.peer_id_str());
            orch.add_node(node).await;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        info!("Connecting mesh...");
        orch.connect_all().await;

        // Check all nodes exist
        let count = orch.node_count().await;
        checks.push(TestCheck::new(
            "nodes_created",
            count == self.node_count,
            self.node_count.to_string(),
            count.to_string(),
        ));

        let passed = checks.iter().all(|c| c.passed);
        ScenarioResult {
            name: self.name().into(),
            passed,
            duration_ms: start.elapsed().as_millis() as u64,
            checks,
            error: None,
        }
    }
}

// ── Scenario 2: Gossipsub Propagation ──────────────────────

pub struct GossipPropagationScenario {
    pub topic: String,
    pub message: String,
}

#[async_trait::async_trait]
impl Scenario for GossipPropagationScenario {
    fn name(&self) -> &str {
        "gossip_propagation"
    }

    async fn run(&self, orch: &TestOrchestrator) -> ScenarioResult {
        let start = std::time::Instant::now();
        let mut checks = Vec::new();

        let topic = gossipsub::IdentTopic::new(&self.topic);

        // Create 3 nodes, subscribe to topic
        for i in 0..3 {
            let port = 22000 + i as u16;
            let mut node = TestNode::new(port);
            node.subscribe(&topic);
            orch.add_node(node).await;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        orch.connect_all().await;

        // Publish on first node
        {
            let mut nodes = orch.nodes.lock().await;
            info!("Publishing '{}' on topic '{}'", self.message, self.topic);
            nodes[0].publish(&topic, self.message.as_bytes());
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // Check node 1 received the message
        let mut nodes = orch.nodes.lock().await;
        let received1 = wait_for_event(
            &mut nodes[1].swarm,
            |e| matches!(e, SwarmEvent::Behaviour(gossipsub::Event::Message { .. })),
            2000,
        )
        .await;
        drop(nodes);

        checks.push(TestCheck::new(
            "gossip_node1",
            received1,
            "message received",
            if received1 {
                "received"
            } else {
                "not received"
            },
        ));

        let passed = checks.iter().all(|c| c.passed);
        ScenarioResult {
            name: self.name().into(),
            passed,
            duration_ms: start.elapsed().as_millis() as u64,
            checks,
            error: None,
        }
    }
}

// ── Scenario 3: Sequential Messages ─────────────────────────

pub struct SequentialMessagesScenario;

#[async_trait::async_trait]
impl Scenario for SequentialMessagesScenario {
    fn name(&self) -> &str {
        "sequential_messages"
    }

    async fn run(&self, orch: &TestOrchestrator) -> ScenarioResult {
        let start = std::time::Instant::now();
        let mut checks = Vec::new();

        let topic = gossipsub::IdentTopic::new("test/seq");

        // Create 2 nodes
        for i in 0..2 {
            let port = 23000 + i as u16;
            let mut node = TestNode::new(port);
            node.subscribe(&topic);
            orch.add_node(node).await;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        orch.connect_all().await;

        // Send 3 messages in sequence
        let messages = vec!["msg1", "msg2", "msg3"];
        for msg in &messages {
            let mut nodes = orch.nodes.lock().await;
            nodes[0].publish(&topic, msg.as_bytes());
            drop(nodes);
            tokio::time::sleep(Duration::from_millis(400)).await;
        }

        // Collect received messages on node 1
        let mut received: Vec<String> = Vec::new();
        let mut nodes = orch.nodes.lock().await;
        use futures::StreamExt;
        for _ in 0..500 {
            tokio::select! {
                event = nodes[1].swarm.next() => {
                    if let Some(SwarmEvent::Behaviour(gossipsub::Event::Message { message, .. })) = event {
                        received.push(String::from_utf8_lossy(&message.data).to_string());
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
        drop(nodes);

        for &msg in &messages {
            let found = received.iter().any(|r| r.as_str() == msg);
            checks.push(TestCheck::new(
                &format!("received_{msg}"),
                found,
                msg,
                if found { msg } else { "missing" },
            ));
        }

        let passed = checks.iter().all(|c| c.passed);
        ScenarioResult {
            name: self.name().into(),
            passed,
            duration_ms: start.elapsed().as_millis() as u64,
            checks,
            error: None,
        }
    }
}

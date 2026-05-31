use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use libp2p::kad::{self, store::MemoryStore};
use libp2p::request_response::{self, Message};
use libp2p::{
    gossipsub, identify, identity, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, Swarm, Transport,
};
use market_protocol::{JobBehaviour, JobOffer, JobResponse};
use p2p_cache::{BlockCache, BlockRequest, BlockResponse};
use reputation_engine::ReputationState;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, trace, warn};
use trust_graph::TrustGraph;

use super::gossip;
use super::HEARTBEAT_INTERVAL_SECS;

/// Combined network behaviour for Flovenet nodes
#[derive(NetworkBehaviour)]
pub struct FlovenetBehaviour {
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub gossipsub: gossipsub::Behaviour,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub job_market: JobBehaviour,
    pub block_cache: p2p_cache::CacheBehaviour,
}

#[allow(clippy::type_complexity)]
pub struct NodeNetwork {
    pub swarm: Swarm<FlovenetBehaviour>,
    pub peer_id: PeerId,
    #[allow(dead_code)]
    pub keypair: identity::Keypair,
    pub listen_addr: Multiaddr,
    pub job_handler: Arc<Mutex<Option<Box<dyn FnMut(JobOffer) -> JobResponse + Send>>>>,
    pub reputation: Arc<RwLock<ReputationState>>,
    pub trust_graph: Arc<RwLock<TrustGraph>>,
    pub block_cache: Arc<BlockCache>,
}

impl NodeNetwork {
    pub async fn set_job_handler<F>(&mut self, handler: F)
    where
        F: FnMut(JobOffer) -> JobResponse + Send + 'static,
    {
        *self.job_handler.lock().await = Some(Box::new(handler));
    }
}

fn build_transport(
    keypair: &identity::Keypair,
    _swarm_key: Option<[u8; 32]>,
) -> Result<libp2p::core::transport::Boxed<(PeerId, libp2p::core::muxing::StreamMuxerBox)>> {
    if _swarm_key.is_some() {
        tracing::warn!("Swarm key (PSK) provided but transport-level PSK not yet implemented");
    }
    let transport = libp2p::tcp::tokio::Transport::new(libp2p::tcp::Config::new().nodelay(true));
    let transport = libp2p::dns::tokio::Transport::system(transport)?;
    let transport = transport
        .upgrade(libp2p::core::upgrade::Version::V1Lazy)
        .authenticate(libp2p::noise::Config::new(keypair)?)
        .multiplex(libp2p::yamux::Config::default())
        .boxed();
    Ok(transport)
}

impl NodeNetwork {
    /// Create and start a new network node
    /// `swarm_key` is an optional 32-byte PSK for private sub-network access.
    pub fn new(
        port: u16,
        keypair: Option<identity::Keypair>,
        swarm_key: Option<[u8; 32]>,
    ) -> Result<Self> {
        let keypair = keypair.unwrap_or_else(identity::Keypair::generate_ed25519);
        let peer_id = PeerId::from(keypair.public());

        let transport = build_transport(&keypair, swarm_key)?;

        let mut kad = kad::Behaviour::new(peer_id, MemoryStore::new(peer_id));
        kad.set_mode(Some(kad::Mode::Server));

        let gs = gossip::create_gossipsub();

        let identify = identify::Behaviour::new(
            identify::Config::new("/flovenet/1.0.0".into(), keypair.public())
                .with_interval(std::time::Duration::from_secs(60)),
        );

        let ping = ping::Behaviour::new(
            ping::Config::new().with_timeout(std::time::Duration::from_secs(20)),
        );

        let job_market = market_protocol::create_job_behaviour();
        let block_cache_behaviour = p2p_cache::create_cache_behaviour();

        let behaviour = FlovenetBehaviour {
            kademlia: kad,
            gossipsub: gs,
            identify,
            ping,
            job_market,
            block_cache: block_cache_behaviour,
        };

        let mut swarm = Swarm::new(
            transport,
            behaviour,
            peer_id,
            libp2p::swarm::Config::with_tokio_executor(),
        );

        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{port}").parse()?;
        swarm.listen_on(listen_addr.clone())?;

        for topic in gossip::all_topics() {
            swarm.behaviour_mut().gossipsub.subscribe(&topic)?;
        }

        info!("Node created: {peer_id}, listening on {listen_addr}");

        let reputation = Arc::new(RwLock::new(ReputationState::new()));
        let trust_graph = Arc::new(RwLock::new(TrustGraph::new()));
        let block_cache = Arc::new(BlockCache::new(1024));

        Ok(Self {
            swarm,
            peer_id,
            keypair,
            listen_addr,
            job_handler: Arc::new(Mutex::new(None)),
            reputation,
            trust_graph,
            block_cache,
        })
    }

    /// Bootstrap Kademlia with known peers
    #[allow(dead_code)]
    pub fn bootstrap_kademlia(&mut self, bootstrap_peers: &[Multiaddr]) -> Result<()> {
        for addr in bootstrap_peers {
            if let Some(peer_id) = addr.iter().last().and_then(|p| match p {
                libp2p::core::multiaddr::Protocol::P2p(h) => Some(h),
                _ => None,
            }) {
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, addr.clone());
            }
        }

        if !bootstrap_peers.is_empty() {
            self.swarm.behaviour_mut().kademlia.bootstrap()?;
        }

        Ok(())
    }

    /// Start the network event loop (runs until shutdown)
    pub async fn run(&mut self) -> Result<()> {
        let mut last_heartbeat = std::time::Instant::now();
        let mut last_reputation_publish = std::time::Instant::now();
        let rep_publish_interval: u64 = HEARTBEAT_INTERVAL_SECS * 3; // every 3 heartbeats

        loop {
            if last_heartbeat.elapsed().as_secs() >= HEARTBEAT_INTERVAL_SECS {
                self.publish_node_status();
                last_heartbeat = std::time::Instant::now();
            }

            if last_reputation_publish.elapsed().as_secs() >= rep_publish_interval {
                self.publish_reputation();
                last_reputation_publish = std::time::Instant::now();
            }

            match self.swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!("Listening on {address}");
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    info!("Connected to {peer_id}");
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    info!("Disconnected from {peer_id}: {cause:?}");
                }
                SwarmEvent::Behaviour(FlovenetBehaviourEvent::Kademlia(event)) => {
                    handle_kademlia_event(event);
                }
                SwarmEvent::Behaviour(FlovenetBehaviourEvent::Gossipsub(event)) => {
                    self.handle_gossip_event(event);
                }
                SwarmEvent::Behaviour(FlovenetBehaviourEvent::Identify(event)) => {
                    handle_identify_event(&mut self.swarm, event);
                }
                SwarmEvent::Behaviour(FlovenetBehaviourEvent::Ping(event)) => {
                    handle_ping_event(event);
                }
                SwarmEvent::Behaviour(FlovenetBehaviourEvent::JobMarket(event)) => {
                    self.handle_job_market_event(event).await;
                }
                SwarmEvent::Behaviour(FlovenetBehaviourEvent::BlockCache(event)) => {
                    self.handle_block_cache_event(event).await;
                }
                _ => {}
            }
        }
    }

    async fn handle_job_market_event(&mut self, event: market_protocol::JobEvent) {
        match event {
            request_response::Event::Message { peer, message, .. } => match message {
                Message::Request {
                    request, channel, ..
                } => {
                    info!("Job offer from {peer}: {:?}", request.job_id);
                    let response = {
                        let mut guard = self.job_handler.lock().await;
                        match guard.as_mut() {
                            Some(handler) => handler(request),
                            None => JobResponse {
                                job_id: request.job_id,
                                accepted: false,
                                reason: Some("no handler registered".into()),
                                result_cid: None,
                            },
                        }
                    };
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .job_market
                        .send_response(channel, response);
                }
                Message::Response {
                    request_id,
                    response,
                    ..
                } => {
                    info!(
                        "Job response for {}: accepted={}",
                        request_id, response.accepted
                    );
                }
            },
            request_response::Event::OutboundFailure { peer, error, .. } => {
                warn!("Outbound job market failure to {peer}: {error:?}");
            }
            request_response::Event::InboundFailure { peer, error, .. } => {
                warn!("Inbound job market failure from {peer}: {error:?}");
            }
            request_response::Event::ResponseSent { peer, .. } => {
                trace!("Job response sent to {peer}");
            }
        }
    }

    async fn handle_block_cache_event(
        &mut self,
        event: request_response::Event<BlockRequest, BlockResponse>,
    ) {
        match event {
            request_response::Event::Message {
                peer: _, message, ..
            } => match message {
                Message::Request {
                    request, channel, ..
                } => {
                    let resp = self.block_cache.handle_request(&request).await;
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .block_cache
                        .send_response(channel, resp);
                }
                Message::Response { response, .. } => {
                    self.block_cache.handle_response(&response).await;
                }
            },
            request_response::Event::OutboundFailure { peer, error, .. } => {
                tracing::warn!("Block cache outbound failure to {peer}: {error:?}");
            }
            request_response::Event::InboundFailure { peer, error, .. } => {
                tracing::warn!("Block cache inbound failure from {peer}: {error:?}");
            }
            request_response::Event::ResponseSent { peer, .. } => {
                tracing::trace!("Block response sent to {peer}");
            }
        }
    }

    /// Request a block from a specific peer.
    #[allow(dead_code)]
    pub async fn request_block(&mut self, peer: PeerId, cid: &str) {
        let req = BlockRequest {
            cid: cid.to_string(),
        };
        self.swarm
            .behaviour_mut()
            .block_cache
            .send_request(&peer, req);
    }

    #[allow(dead_code)]
    pub async fn send_job_offer(&mut self, peer: PeerId, offer: JobOffer) {
        self.swarm
            .behaviour_mut()
            .job_market
            .send_request(&peer, offer);
    }

    fn publish_node_status(&mut self) {
        let rep_score = self
            .reputation
            .blocking_read()
            .get_score(&self.peer_id.to_string())
            .cloned();
        let status = serde_json::json!({
            "peer_id": self.peer_id.to_string(),
            "timestamp_secs": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "reputation_score": rep_score.as_ref().map(|s| s.score),
            "uptime_pct": rep_score.as_ref().map(|s| s.uptime_pct),
        });

        if let Ok(data) = serde_json::to_vec(&status) {
            let _ = self
                .swarm
                .behaviour_mut()
                .gossipsub
                .publish(gossip::status_topic(), data);
        }
    }

    /// Publish this node's reputation score via Gossipsub.
    fn publish_reputation(&mut self) {
        let rep = self.reputation.blocking_read().clone();
        if let Ok(data) = serde_json::to_vec(&rep) {
            let _ = self
                .swarm
                .behaviour_mut()
                .gossipsub
                .publish(gossip::reputation_topic(), data);
        }
    }

    fn handle_gossip_event(&self, event: gossipsub::Event) {
        if let gossipsub::Event::Message { message, .. } = event {
            match message.topic.as_str() {
                gossip::TOPIC_REPUTATION => {
                    if let Ok(remote) = serde_json::from_slice::<ReputationState>(&message.data) {
                        self.reputation.blocking_write().merge(&remote);
                    }
                }
                gossip::TOPIC_TRUST_EDGE => {
                    if let Ok(edge) =
                        serde_json::from_slice::<trust_graph::TrustEdge>(&message.data)
                    {
                        self.trust_graph.blocking_write().add_edge(edge);
                    }
                }
                topic => {
                    let data_str = String::from_utf8_lossy(&message.data);
                    trace!("Gossip message on {topic}: {data_str:.100}");
                }
            }
        }
    }

    /// Publish a trust edge via Gossipsub.
    #[allow(dead_code)]
    pub fn publish_trust_edge(&mut self, edge: trust_graph::TrustEdge) {
        if let Ok(data) = serde_json::to_vec(&edge) {
            let _ = self
                .swarm
                .behaviour_mut()
                .gossipsub
                .publish(gossip::trust_edge_topic(), data);
        }
    }
}

fn handle_kademlia_event(event: kad::Event) {
    match event {
        kad::Event::RoutingUpdated { peer, .. } => {
            trace!("Kademlia routing updated for {peer}");
        }
        kad::Event::InboundRequest { request } => {
            trace!("Kademlia inbound request: {:?}", request);
        }
        _ => {}
    }
}

fn handle_identify_event(swarm: &mut Swarm<FlovenetBehaviour>, event: identify::Event) {
    if let identify::Event::Received { peer_id, info, .. } = event {
        for addr in info.listen_addrs {
            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
        }
    }
}

fn handle_ping_event(event: ping::Event) {
    if let ping::Event {
        peer,
        result: Ok(rtt),
        ..
    } = event
    {
        trace!("Ping to {peer}: {rtt:?}");
    }
}

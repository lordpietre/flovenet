pub mod discovery;
pub mod gossip;
pub mod swarm;

pub use swarm::NodeNetwork;

#[allow(dead_code)]
pub const GATEWAY_KEY: &str = "/flovenet/gateway/1.0.0";
#[allow(dead_code)]
pub const GATEWAY_EXPIRE_SECS: u64 = 120;
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Load an optional swarm key file (PSK) for private sub-network support.
/// The key file should contain 32 bytes of raw key material.
/// Returns None if the path is empty or the file cannot be read.
pub fn load_swarm_key(path: Option<&str>) -> Option<[u8; 32]> {
    let path = path?;
    let data = std::fs::read(path).ok()?;
    let mut key = [0u8; 32];
    if data.len() >= 32 {
        key.copy_from_slice(&data[..32]);
        Some(key)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_swarm_key_none() {
        assert_eq!(load_swarm_key(None), None);
    }

    #[test]
    fn test_load_swarm_key_invalid_path() {
        assert_eq!(load_swarm_key(Some("/nonexistent/key.bin")), None);
    }

    #[test]
    fn test_load_swarm_key_too_short() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.key");
        std::fs::write(&path, [0u8; 16]).unwrap();
        assert_eq!(load_swarm_key(Some(path.to_str().unwrap())), None);
    }

    #[test]
    fn test_load_swarm_key_exact_32() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exact.key");
        let key_bytes: [u8; 32] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ];
        std::fs::write(&path, key_bytes).unwrap();
        let loaded = load_swarm_key(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(loaded, key_bytes);
    }

    #[test]
    fn test_load_swarm_key_truncates_extra() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.key");
        let data = vec![0xabu8; 64];
        std::fs::write(&path, &data).unwrap();
        let loaded = load_swarm_key(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(loaded.len(), 32);
        assert_eq!(loaded, [0xab; 32]);
    }

    #[test]
    fn test_gateway_constants() {
        assert_eq!(GATEWAY_KEY, "/flovenet/gateway/1.0.0");
        assert_eq!(GATEWAY_EXPIRE_SECS, 120);
        assert_eq!(HEARTBEAT_INTERVAL_SECS, 30);
    }

    // --- Real P2P communication test ---
    //
    // Creates two gossipsub-only swarms on localhost, connects them,
    // and verifies that a message published on one is received by the other.
    //
    // We build minimal swarms (not full NodeNetwork) so we can drive
    // the event loop precisely.

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use futures::StreamExt;
    use libp2p::gossipsub::{self, MessageAuthenticity, MessageId};
    use libp2p::swarm::SwarmEvent;
    use libp2p::{identity, Multiaddr, PeerId, Swarm, Transport};

    /// Build a minimal gossipsub-only swarm for testing.
    /// Uses TCP + Noise + Yamux on localhost:0 (OS-assigned port).
    fn build_test_swarm() -> Swarm<gossipsub::Behaviour> {
        let key = identity::Keypair::generate_ed25519();

        let transport =
            libp2p::tcp::tokio::Transport::new(libp2p::tcp::Config::new().nodelay(true))
                .upgrade(libp2p::core::upgrade::Version::V1Lazy)
                .authenticate(libp2p::noise::Config::new(&key).unwrap())
                .multiplex(libp2p::yamux::Config::default())
                .boxed();

        let msg_id_fn = |msg: &gossipsub::Message| {
            let data = &msg.data[..msg.data.len().min(64)];
            let hash = super::gossip::quickhash::xxh3_64(data);
            MessageId::from(hash.to_le_bytes().to_vec())
        };

        let config = gossipsub::ConfigBuilder::default()
            .heartbeat_initial_delay(Duration::from_millis(100))
            .heartbeat_interval(Duration::from_millis(500))
            .message_id_fn(msg_id_fn)
            .validation_mode(gossipsub::ValidationMode::Anonymous)
            .build()
            .unwrap();

        let behaviour = gossipsub::Behaviour::new(MessageAuthenticity::Anonymous, config).unwrap();

        let mut swarm = Swarm::new(
            transport,
            behaviour,
            PeerId::from(key.public()),
            libp2p::swarm::Config::with_tokio_executor(),
        );

        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
        swarm.listen_on(addr).unwrap();

        swarm
    }

    #[tokio::test]
    async fn test_p2p_gossip_message_exchange() {
        let mut swarm_a = build_test_swarm();
        let (mut swarm_b, peer_b) = {
            let b = build_test_swarm();
            let pid = *b.local_peer_id();
            (b, pid)
        };

        let topic = gossipsub::IdentTopic::new("test/flovenet-p2p");
        swarm_a.behaviour_mut().subscribe(&topic).unwrap();
        swarm_b.behaviour_mut().subscribe(&topic).unwrap();

        // 1. get listen address for B
        let mut addr_b = None;
        for _ in 0..100 {
            tokio::select! {
                event = swarm_a.select_next_some() => { drop(event); }
                event = swarm_b.select_next_some() => {
                    if let SwarmEvent::NewListenAddr { address, .. } = event {
                        addr_b = Some(address);
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => break,
            }
            if addr_b.is_some() {
                break;
            }
        }
        let addr_b = addr_b.expect("swarm B got no listen addr");

        // 2. dial A → B
        swarm_a
            .dial(addr_b.with(libp2p::multiaddr::Protocol::P2p(peer_b)))
            .unwrap();

        // 3. poll both until a connection is established
        for _ in 0..300 {
            tokio::select! {
                event = swarm_a.select_next_some() => { drop(event); }
                event = swarm_b.select_next_some() => { drop(event); }
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
            // Check if B has peers in the gossipsub mesh
            // (a WeakNotification already means the connection itself succeeded)
        }

        // 4. wait for at least one gossipsub heartbeat to form the mesh
        tokio::time::sleep(Duration::from_millis(600)).await;
        for _ in 0..100 {
            tokio::select! {
                event = swarm_a.select_next_some() => { drop(event); }
                event = swarm_b.select_next_some() => { drop(event); }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }

        // 5. publish from A (should now have peers in mesh)
        let msg_content = "hello from P2P test";
        swarm_a
            .behaviour_mut()
            .publish(topic.clone(), msg_content.as_bytes())
            .expect("publish should succeed");

        // 7. poll both until B receives
        let received = Arc::new(AtomicBool::new(false));
        let r = received.clone();

        for _ in 0..1000 {
            tokio::select! {
                event = swarm_a.select_next_some() => { drop(event); }
                event = swarm_b.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message { message, .. }) = event {
                        if String::from_utf8_lossy(&message.data) == msg_content {
                            r.store(true, Ordering::SeqCst);
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            if received.load(Ordering::SeqCst) {
                break;
            }
        }

        assert!(
            received.load(Ordering::SeqCst),
            "swarm B should receive message from swarm A"
        );
    }
}

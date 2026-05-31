pub mod keystore;

use serde::{Deserialize, Serialize};

pub use keystore::KeyStore;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PeerId(pub String);

impl PeerId {
    pub fn from_public_key_bytes(pub_key: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(pub_key);
        let mh = multihash::Multihash::wrap(0x12, &hash).expect("sha2-256 multihash");
        let cid = cid::Cid::new_v1(0x55, mh);
        PeerId(cid.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub peer_id: PeerId,
    pub display_name: String,
    pub bio: Option<String>,
    pub avatar_cid: Option<String>,
    pub public_key: Vec<u8>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Profile {
    pub fn new(peer_id: PeerId, display_name: String, public_key: Vec<u8>) -> Self {
        Profile {
            peer_id,
            display_name,
            bio: None,
            avatar_cid: None,
            public_key,
            created_at: chrono::Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::generate_keypair;

    #[test]
    fn test_peer_id_from_pub_key() {
        let kp = generate_keypair();
        let pk_bytes = kp.verifying_key().to_bytes().to_vec();
        let pid = PeerId::from_public_key_bytes(&pk_bytes);
        assert!(!pid.0.is_empty());
        assert!(pid.0.starts_with("bafk"));
    }

    #[test]
    fn test_profile_new() {
        let kp = generate_keypair();
        let pk_bytes = kp.verifying_key().to_bytes().to_vec();
        let pid = PeerId::from_public_key_bytes(&pk_bytes);
        let profile = Profile::new(pid.clone(), "test_user".into(), pk_bytes);
        assert_eq!(profile.peer_id, pid);
        assert_eq!(profile.display_name, "test_user");
    }
}

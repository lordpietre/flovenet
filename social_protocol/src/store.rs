use std::sync::Arc;

use storage::{StorageBackend, StorageOpts};

use crate::post::Post;
use crate::profile::Profile;

/// A typed store for social data, backed by a `StorageBackend`.
pub struct SocialStore {
    backend: Arc<dyn StorageBackend>,
}

impl SocialStore {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self { backend }
    }

    /// Store content and return its CID.
    pub async fn store(&self, bytes: &[u8]) -> Result<String, storage::error::StorageError> {
        let cid = self
            .backend
            .add(
                bytes,
                StorageOpts {
                    pin: true,
                    ttl: None,
                },
            )
            .await?;
        Ok(cid.to_string())
    }

    /// Retrieve content by its CID
    pub async fn get(&self, cid: &str) -> Result<Option<Vec<u8>>, storage::error::StorageError> {
        let parsed: storage::CID = match cid.parse() {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        match self.backend.get(&parsed).await {
            Ok(data) => Ok(Some(data)),
            Err(storage::error::StorageError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    // ── Profiles ──

    pub async fn put_profile(&self, profile: &Profile) -> Result<(), storage::error::StorageError> {
        let bytes = serde_json::to_vec(profile)
            .map_err(|e| storage::error::StorageError::Backend(e.to_string()))?;
        let cid = self
            .backend
            .add(
                &bytes,
                StorageOpts {
                    pin: true,
                    ttl: None,
                },
            )
            .await?;
        let idx_key = format!("profile:{}", profile.peer_id);
        self.backend
            .add(
                idx_key.as_bytes(),
                StorageOpts {
                    pin: true,
                    ttl: None,
                },
            )
            .await?;
        tracing::info!(peer_id = %profile.peer_id, cid = %cid, "profile stored");
        Ok(())
    }

    pub async fn get_profile(
        &self,
        peer_id: &str,
    ) -> Result<Option<Profile>, storage::error::StorageError> {
        let pins = self.backend.ls_pins().await?;
        for pin in &pins {
            if let Ok(data) = self.backend.get(&pin.cid).await {
                if let Ok(profile) = serde_json::from_slice::<Profile>(&data) {
                    if profile.peer_id == peer_id {
                        return Ok(Some(profile));
                    }
                }
            }
        }
        Ok(None)
    }

    // ── Posts ──

    pub async fn put_post(&self, post: &Post) -> Result<(), storage::error::StorageError> {
        let bytes = serde_json::to_vec(post)
            .map_err(|e| storage::error::StorageError::Backend(e.to_string()))?;
        let cid = self
            .backend
            .add(
                &bytes,
                StorageOpts {
                    pin: true,
                    ttl: None,
                },
            )
            .await?;
        let idx_key = format!("post:{}", post.cid);
        self.backend
            .add(
                idx_key.as_bytes(),
                StorageOpts {
                    pin: true,
                    ttl: None,
                },
            )
            .await?;
        tracing::info!(cid = %cid, "post stored");
        Ok(())
    }

    pub async fn get_post(&self, cid: &str) -> Result<Option<Post>, storage::error::StorageError> {
        let pins = self.backend.ls_pins().await?;
        for pin in &pins {
            if let Ok(data) = self.backend.get(&pin.cid).await {
                if let Ok(post) = serde_json::from_slice::<Post>(&data) {
                    if post.cid == cid {
                        return Ok(Some(post));
                    }
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post::Post;
    use storage::local::LocalBackend;

    fn test_store() -> SocialStore {
        let dir = tempfile::tempdir().unwrap();
        let local = LocalBackend::new(dir.path());
        SocialStore::new(Arc::new(local))
    }

    #[tokio::test]
    async fn test_profile_roundtrip() {
        let store = test_store();
        let profile = Profile {
            peer_id: "peer1".into(),
            display_name: "Alice".into(),
            bio: Some("Hello".into()),
            avatar_cid: None,
            follower_count: 0,
            following_count: 0,
            post_count: 0,
            reputation: None,
            public_key: vec![1, 2, 3],
        };

        store.put_profile(&profile).await.unwrap();
        let retrieved = store.get_profile("peer1").await.unwrap().unwrap();
        assert_eq!(retrieved.display_name, "Alice");
    }

    #[tokio::test]
    async fn test_post_roundtrip() {
        let store = test_store();
        let post = Post {
            cid: "post-1".into(),
            author: "peer1".into(),
            content: "Hello world".into(),
            media: vec![],
            parent: None,
            reply_count: 0,
            like_count: 0,
            timestamp: chrono::Utc::now(),
            signature: vec![1, 2, 3],
        };

        store.put_post(&post).await.unwrap();
        let retrieved = store.get_post("post-1").await.unwrap().unwrap();
        assert_eq!(retrieved.content, "Hello world");
    }

    #[tokio::test]
    async fn test_store_get() {
        let store = test_store();
        let data = b"hello flovenet";
        let cid = store.store(data).await.unwrap();
        let retrieved = store.get(&cid).await.unwrap().unwrap();
        assert_eq!(retrieved, data);
    }
}

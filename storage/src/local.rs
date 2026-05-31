use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use tokio::fs;
use tracing::info;

use crate::error::StorageError;
use crate::{PinPolicy, PinnedItem, StorageBackend, StorageOpts, CID};

/// Filesystem-based storage backend.
/// Stores blobs at `<base_dir>/data/<cid>` and pin metadata at `<base_dir>/pins.json`.
pub struct LocalBackend {
    base_dir: PathBuf,
    pins_path: PathBuf,
}

impl LocalBackend {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir: PathBuf = base_dir.into();
        let pins_path = base_dir.join("pins.json");
        Self {
            base_dir,
            pins_path,
        }
    }

    async fn data_dir(&self) -> PathBuf {
        let dir = self.base_dir.join("data");
        fs::create_dir_all(&dir).await.expect("create data dir");
        dir
    }

    async fn load_pins(&self) -> HashMap<String, PinnedItem> {
        match fs::read_to_string(&self.pins_path).await {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    async fn save_pins(&self, pins: &HashMap<String, PinnedItem>) -> Result<(), StorageError> {
        let content = serde_json::to_string(pins)
            .map_err(|e| StorageError::Backend(format!("failed to serialize pins: {e}")))?;
        fs::write(&self.pins_path, content).await?;
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for LocalBackend {
    async fn add(&self, bytes: &[u8], opts: StorageOpts) -> Result<CID, StorageError> {
        let cid = compute_cid(bytes);
        let data_dir = self.data_dir().await;
        let path = data_dir.join(cid.to_string());

        fs::write(&path, bytes).await?;

        if opts.pin {
            self.pin(&cid, PinPolicy::Permanent).await?;
        }

        info!(cid = %cid, size = bytes.len(), "stored locally");
        Ok(cid)
    }

    async fn get(&self, cid: &CID) -> Result<Vec<u8>, StorageError> {
        let data_dir = self.data_dir().await;
        let path = data_dir.join(cid.to_string());

        if !path.exists() {
            return Err(StorageError::NotFound(cid.to_string()));
        }

        let bytes = fs::read(&path).await?;
        Ok(bytes)
    }

    async fn pin(&self, cid: &CID, policy: PinPolicy) -> Result<(), StorageError> {
        let mut pins = self.load_pins().await;
        pins.insert(
            cid.to_string(),
            PinnedItem {
                cid: *cid,
                policy,
                pinned_at: chrono::Utc::now(),
            },
        );
        self.save_pins(&pins).await
    }

    async fn unpin(&self, cid: &CID) -> Result<(), StorageError> {
        let mut pins = self.load_pins().await;
        pins.remove(&cid.to_string());
        self.save_pins(&pins).await
    }

    async fn ls_pins(&self) -> Result<Vec<PinnedItem>, StorageError> {
        let pins = self.load_pins().await;
        Ok(pins.into_values().collect())
    }

    async fn delete(&self, cid: &CID) -> Result<(), StorageError> {
        let data_dir = self.data_dir().await;
        let path = data_dir.join(cid.to_string());

        if path.exists() {
            fs::remove_file(&path).await?;
        }

        self.unpin(cid).await?;
        info!(cid = %cid, "deleted locally");
        Ok(())
    }
}

/// Compute a CIDv1 from raw bytes using SHA2-256 multihash.
fn compute_cid(bytes: &[u8]) -> CID {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mh = multihash::Multihash::wrap(0x12, &digest).expect("valid multihash");
    CID::new_v1(0x55, mh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_add_get() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(dir.path());

        let data = b"hello flovenet";
        let opts = StorageOpts {
            pin: true,
            ttl: None,
        };

        let cid = backend.add(data, opts).await.unwrap();
        let retrieved = backend.get(&cid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_local_pin_unpin() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(dir.path());

        let data = b"pin test data";
        let cid = backend
            .add(
                data,
                StorageOpts {
                    pin: true,
                    ttl: None,
                },
            )
            .await
            .unwrap();

        let pins = backend.ls_pins().await.unwrap();
        assert!(pins.iter().any(|p| p.cid == cid));

        backend.unpin(&cid).await.unwrap();
        let pins = backend.ls_pins().await.unwrap();
        assert!(!pins.iter().any(|p| p.cid == cid));
    }

    #[tokio::test]
    async fn test_local_get_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(dir.path());

        let fake_cid = compute_cid(b"does not exist");
        let result = backend.get(&fake_cid).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_local_delete() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(dir.path());

        let data = b"delete me";
        let cid = backend
            .add(
                data,
                StorageOpts {
                    pin: true,
                    ttl: None,
                },
            )
            .await
            .unwrap();

        backend.delete(&cid).await.unwrap();
        let result = backend.get(&cid).await;
        assert!(result.is_err());
    }
}

use async_trait::async_trait;
use tracing::warn;

use crate::error::StorageError;
use crate::{PinPolicy, PinnedItem, StorageBackend, StorageOpts, CID};

/// A storage backend that composites multiple backends with a tiered policy.
///
/// - Write: data is stored in all available backends for redundancy.
/// - Read: tries backends in order (fastest first), falls back on failure.
/// - Delete: removes from all backends.
pub struct HybridBackend {
    tiers: Vec<Box<dyn StorageBackend + Send + Sync>>,
}

impl HybridBackend {
    /// Create a hybrid backend with tiers ordered from fastest to slowest.
    /// Example: [LocalBackend, IpfsBackend, S3Backend]
    pub fn new(tiers: Vec<Box<dyn StorageBackend + Send + Sync>>) -> Self {
        Self { tiers }
    }

    /// Add a backend to the end of the tier list.
    pub fn push_backend(&mut self, backend: Box<dyn StorageBackend + Send + Sync>) {
        self.tiers.push(backend);
    }

    /// Number of configured backends.
    pub fn tier_count(&self) -> usize {
        self.tiers.len()
    }
}

#[async_trait]
impl StorageBackend for HybridBackend {
    async fn add(&self, bytes: &[u8], opts: StorageOpts) -> Result<CID, StorageError> {
        let mut last_cid = None;
        let mut last_err = None;

        for (i, tier) in self.tiers.iter().enumerate() {
            match tier.add(bytes, opts.clone()).await {
                Ok(cid) => {
                    last_cid = Some(cid);
                }
                Err(e) => {
                    warn!(tier = i, error = %e, "hybrid add failed on tier");
                    last_err = Some(e);
                }
            }
        }

        last_cid.ok_or_else(|| {
            last_err.unwrap_or_else(|| StorageError::Backend("all tiers failed on add".into()))
        })
    }

    async fn get(&self, cid: &CID) -> Result<Vec<u8>, StorageError> {
        let mut last_err = None;

        for (i, tier) in self.tiers.iter().enumerate() {
            match tier.get(cid).await {
                Ok(data) => {
                    if i > 0 {
                        // Promote to faster tiers on successful read (eventual migration)
                        let opts = StorageOpts {
                            pin: true,
                            ttl: None,
                        };
                        for j in 0..i {
                            if let Err(e) = self.tiers[j].add(&data, opts.clone()).await {
                                warn!(from = i, to = j, %cid, error = %e, "hybrid promote failed");
                            }
                        }
                    }
                    return Ok(data);
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| StorageError::NotFound(cid.to_string())))
    }

    async fn pin(&self, cid: &CID, policy: PinPolicy) -> Result<(), StorageError> {
        let mut last_err = None;
        for tier in &self.tiers {
            if let Err(e) = tier.pin(cid, policy.clone()).await {
                warn!(%cid, error = %e, "hybrid pin failed on tier");
                last_err = Some(e);
            }
        }
        last_err
            .map(|_| ())
            .ok_or_else(|| StorageError::Backend("all tiers failed on pin".into()))
    }

    async fn unpin(&self, cid: &CID) -> Result<(), StorageError> {
        let mut last_err = None;
        for tier in &self.tiers {
            if let Err(e) = tier.unpin(cid).await {
                warn!(%cid, error = %e, "hybrid unpin failed on tier");
                last_err = Some(e);
            }
        }
        last_err
            .map(|_| ())
            .ok_or_else(|| StorageError::Backend("all tiers failed on unpin".into()))
    }

    async fn ls_pins(&self) -> Result<Vec<PinnedItem>, StorageError> {
        let mut all = Vec::new();
        for tier in &self.tiers {
            if let Ok(items) = tier.ls_pins().await {
                all.extend(items);
            }
        }
        // Deduplicate by CID
        let mut seen = std::collections::HashSet::new();
        all.retain(|item| seen.insert(item.cid));
        Ok(all)
    }

    async fn delete(&self, cid: &CID) -> Result<(), StorageError> {
        let mut last_err = None;
        for tier in &self.tiers {
            if let Err(e) = tier.delete(cid).await {
                warn!(%cid, error = %e, "hybrid delete failed on tier");
                last_err = Some(e);
            }
        }
        last_err
            .map(|_| ())
            .ok_or_else(|| StorageError::Backend("all tiers failed on delete".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalBackend;

    #[tokio::test]
    async fn test_hybrid_add_get() {
        let dir = tempfile::tempdir().unwrap();
        let local1 = LocalBackend::new(dir.path().join("tier1"));
        let local2 = LocalBackend::new(dir.path().join("tier2"));
        let hybrid = HybridBackend::new(vec![Box::new(local1), Box::new(local2)]);

        let data = b"hello hybrid world";
        let opts = StorageOpts {
            pin: false,
            ttl: None,
        };
        let cid = hybrid.add(data, opts).await.unwrap();
        let retrieved = hybrid.get(&cid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_hybrid_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let fast = LocalBackend::new(dir.path().join("fast"));
        let fallback = LocalBackend::new(dir.path().join("fallback"));

        // Store only in fallback
        let data = b"fallback data";
        let opts = StorageOpts {
            pin: false,
            ttl: None,
        };
        let cid = fallback.add(data, opts.clone()).await.unwrap();

        let hybrid = HybridBackend::new(vec![Box::new(fast), Box::new(fallback)]);

        // Should find it via fallback tier
        let retrieved = hybrid.get(&cid).await.unwrap();
        assert_eq!(retrieved, data);
    }
}

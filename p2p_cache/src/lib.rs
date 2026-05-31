use std::collections::HashSet;
use std::sync::Arc;

use libp2p::request_response::{self, ProtocolSupport};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};
use storage::CID;
use tokio::sync::RwLock;

/// A request for a content block by CID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockRequest {
    pub cid: String,
}

/// Response containing the requested block data, or empty if not found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockResponse {
    pub cid: String,
    pub data: Vec<u8>,
    pub found: bool,
}

/// Type alias for the libp2p request-response behaviour for block exchange.
pub type CacheBehaviour = request_response::json::Behaviour<BlockRequest, BlockResponse>;

/// The protocol ID for block exchange.
const BLOCK_PROTOCOL: StreamProtocol = StreamProtocol::new("/flovenet/block/1.0.0");

/// Create a new cache behaviour for block exchange.
pub fn create_cache_behaviour() -> CacheBehaviour {
    request_response::json::Behaviour::new(
        [(BLOCK_PROTOCOL, ProtocolSupport::Full)],
        request_response::Config::default(),
    )
}

/// In-memory block cache shared between peers.
/// Stores recently accessed content blocks keyed by CID.
pub struct BlockCache {
    blocks: Arc<RwLock<HashSet<String>>>,
    max_entries: usize,
    storage: Arc<RwLock<Option<Box<dyn storage::StorageBackend + Send + Sync>>>>,
}

impl BlockCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            blocks: Arc::new(RwLock::new(HashSet::new())),
            max_entries,
            storage: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the underlying storage backend for persistent reads.
    pub async fn set_storage(&self, backend: Box<dyn storage::StorageBackend + Send + Sync>) {
        *self.storage.write().await = Some(backend);
    }

    /// Check if a block is cached locally.
    pub async fn has_block(&self, cid: &str) -> bool {
        self.blocks.read().await.contains(cid)
    }

    /// Track a CID as locally available.
    pub async fn add_block(&self, cid: &str) {
        let mut blocks = self.blocks.write().await;
        if blocks.len() >= self.max_entries {
            // Simple eviction: remove one arbitrary entry
            if let Some(oldest) = blocks.iter().next().cloned() {
                blocks.remove(&oldest);
            }
        }
        blocks.insert(cid.to_string());
    }

    /// Get a block: first from cache, then from storage backend.
    pub async fn get_block(&self, cid: &str) -> Option<Vec<u8>> {
        // For now, try storage backend directly (cache tracks availability)
        let storage = self.storage.read().await;
        if let Some(ref backend) = *storage {
            if let Ok(cid_parsed) = cid.parse::<CID>() {
                if let Ok(data) = backend.get(&cid_parsed).await {
                    return Some(data);
                }
            }
        }
        None
    }

    /// Handle an incoming block request.
    /// Returns the block data if available locally.
    pub async fn handle_request(&self, request: &BlockRequest) -> BlockResponse {
        let cid = &request.cid;
        let found = self.has_block(cid).await;
        let data = if found {
            self.get_block(cid).await.unwrap_or_default()
        } else {
            Vec::new()
        };
        BlockResponse {
            cid: cid.clone(),
            data,
            found,
        }
    }

    /// Process a received block response — cache the data.
    pub async fn handle_response(&self, response: &BlockResponse) {
        if response.found {
            self.add_block(&response.cid).await;
        }
    }

    /// Number of cached blocks.
    pub async fn cache_size(&self) -> usize {
        self.blocks.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_request_response_serde() {
        let req = BlockRequest {
            cid: "bafkqaaa".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: BlockRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.cid, "bafkqaaa");

        let resp = BlockResponse {
            cid: "bafkqaaa".into(),
            data: vec![1, 2, 3],
            found: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: BlockResponse = serde_json::from_str(&json).unwrap();
        assert!(deserialized.found);
    }

    #[tokio::test]
    async fn test_block_cache_add_has() {
        let cache = BlockCache::new(100);
        assert!(!cache.has_block("test-cid").await);
        cache.add_block("test-cid").await;
        assert!(cache.has_block("test-cid").await);
    }

    #[tokio::test]
    async fn test_block_cache_eviction() {
        let cache = BlockCache::new(2);
        cache.add_block("a").await;
        cache.add_block("b").await;
        assert!(cache.has_block("a").await);
        assert!(cache.has_block("b").await);
        cache.add_block("c").await; // should evict one
        let size = cache.cache_size().await;
        assert_eq!(size, 2);
    }

    #[tokio::test]
    async fn test_handle_request_not_found() {
        let cache = BlockCache::new(100);
        let req = BlockRequest {
            cid: "nonexistent".into(),
        };
        let resp = cache.handle_request(&req).await;
        assert!(!resp.found);
        assert!(resp.data.is_empty());
    }

    #[tokio::test]
    async fn test_handle_response_caches() {
        let cache = BlockCache::new(100);
        let resp = BlockResponse {
            cid: "new-cid".into(),
            data: vec![10, 20, 30],
            found: true,
        };
        cache.handle_response(&resp).await;
        assert!(cache.has_block("new-cid").await);
    }

    #[tokio::test]
    async fn test_handle_response_not_found_does_not_cache() {
        let cache = BlockCache::new(100);
        let resp = BlockResponse {
            cid: "not-found".into(),
            data: vec![],
            found: false,
        };
        cache.handle_response(&resp).await;
        assert!(!cache.has_block("not-found").await);
    }

    #[tokio::test]
    async fn test_duplicate_add_does_not_overflow() {
        let cache = BlockCache::new(10);
        for _ in 0..100 {
            cache.add_block("same-cid").await;
        }
        // Should still have just one entry
        assert_eq!(cache.cache_size().await, 1);
    }

    #[tokio::test]
    async fn test_cache_with_zero_max() {
        let cache = BlockCache::new(0);
        cache.add_block("a").await;
        // With max=0, eviction always tries to remove (empty set), then inserts
        cache.add_block("b").await;
        // After two inserts, each eviction removes the single element,
        // so size oscillates. Final size could be 1 or 0 depending on eviction.
        assert!(cache.cache_size().await <= 1);
    }

    #[tokio::test]
    async fn test_get_block_no_storage() {
        let cache = BlockCache::new(100);
        cache.add_block("test-cid").await;
        assert!(cache.has_block("test-cid").await);
        // Without storage, get_block returns None
        assert!(cache.get_block("test-cid").await.is_none());
    }

    #[test]
    fn test_create_cache_behaviour() {
        let _behaviour = create_cache_behaviour();
        // just ensure it doesn't panic
    }

    #[test]
    fn test_block_response_found_propagation() {
        let resp = BlockResponse {
            cid: "abc".into(),
            data: vec![1, 2, 3],
            found: true,
        };
        assert!(resp.found);
        assert_eq!(resp.data.len(), 3);
    }
}

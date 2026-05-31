use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::error::StorageError;
use crate::{PinPolicy, PinnedItem, StorageBackend, StorageOpts, CID};

/// S3-compatible storage backend (MinIO / AWS S3).
/// Uses raw HTTP REST calls to the S3 API — no heavy SDK dependency.
pub struct S3Backend {
    endpoint: String,
    bucket: String,
    #[allow(dead_code)]
    region: String,
    /// Base64-encoded "access_key:secret_key" for Basic auth
    auth_header: String,
    /// In-memory pin tracking (S3 has no native pin concept)
    pins: Arc<std::sync::Mutex<HashMap<String, PinnedItem>>>,
    /// Whether the backend is available (connectivity check)
    available: Arc<AtomicBool>,
}

impl S3Backend {
    /// Create a new S3 backend.
    ///
    /// For MinIO: endpoint = "http://localhost:9000", region = "us-east-1"
    /// For AWS S3: endpoint = "https://s3.<region>.amazonaws.com"
    pub fn new(
        endpoint: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
    ) -> Self {
        let auth =
            base64::engine::general_purpose::STANDARD.encode(format!("{access_key}:{secret_key}"));
        let auth_header = format!("Basic {auth}");
        Self {
            endpoint,
            bucket,
            region,
            auth_header,
            pins: Arc::new(std::sync::Mutex::new(HashMap::new())),
            available: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Check connectivity by listing the bucket.
    pub async fn check_connectivity(&self) -> bool {
        let url = format!("{}/{}", self.endpoint, self.bucket);
        let result = self.s3_request("GET", &url, None, None).await;
        let available = result.is_ok();
        self.available.store(available, Ordering::SeqCst);
        available
    }

    /// Whether the backend believes it's available.
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }

    /// Object key for a CID
    fn key_for(cid: &CID) -> String {
        cid.to_string()
    }

    /// Make an S3 API request with method-specific builder.
    /// For MinIO (non-AWS), we use path-style requests with basic auth.
    async fn s3_request(
        &self,
        method: &str,
        url: &str,
        body: Option<Vec<u8>>,
        content_type: Option<&str>,
    ) -> Result<Vec<u8>, StorageError> {
        let url_owned = url.to_string();
        let auth_header = self.auth_header.clone();
        let method_owned = method.to_string();
        let ct_owned = content_type
            .unwrap_or("application/octet-stream")
            .to_string();

        tokio::task::spawn_blocking(move || -> Result<Vec<u8>, StorageError> {
            // Build request with method-specific helper, always set body
            let body_bytes = body.unwrap_or_default();
            let req_builder = match method_owned.as_str() {
                "GET" => attohttpc::get(&url_owned).bytes(body_bytes),
                "PUT" => attohttpc::put(&url_owned).bytes(body_bytes),
                "DELETE" => attohttpc::delete(&url_owned).bytes(body_bytes),
                "HEAD" => attohttpc::head(&url_owned).bytes(body_bytes),
                _ => {
                    return Err(StorageError::Backend(format!(
                        "unsupported method: {method_owned}"
                    )));
                }
            };

            let resp = req_builder
                .header("Authorization", &auth_header)
                .header("Content-Type", &ct_owned)
                .send()
                .map_err(|e| StorageError::Backend(format!("S3 request failed: {e}")))?;

            let status = resp.status();
            let resp_body = resp
                .bytes()
                .map_err(|e| StorageError::Backend(format!("S3 read response failed: {e}")))?;

            if status.is_success() || status == 404 {
                Ok(resp_body.to_vec())
            } else {
                let msg = String::from_utf8_lossy(&resp_body);
                Err(StorageError::Backend(format!(
                    "S3 {method_owned} {url_owned}: {status} - {msg}"
                )))
            }
        })
        .await
        .map_err(|e| StorageError::Backend(format!("blocking task failed: {e}")))?
    }
}

#[async_trait]
impl StorageBackend for S3Backend {
    async fn add(&self, bytes: &[u8], opts: StorageOpts) -> Result<CID, StorageError> {
        // Compute CID locally (SHA2-256 → CIDv1)
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mh = multihash::Multihash::wrap(0x12, &digest)
            .map_err(|e| StorageError::Backend(format!("multihash error: {e}")))?;
        let cid = CID::new_v1(0x55, mh);

        let key = Self::key_for(&cid);
        let url = format!("{}/{}/{}", self.endpoint, self.bucket, key);

        self.s3_request("PUT", &url, Some(bytes.to_vec()), None)
            .await?;

        if opts.pin {
            self.pin(&cid, PinPolicy::Permanent).await?;
        }

        info!(%cid, bucket = %self.bucket, "stored on S3");
        Ok(cid)
    }

    async fn get(&self, cid: &CID) -> Result<Vec<u8>, StorageError> {
        let key = Self::key_for(cid);
        let url = format!("{}/{}/{}", self.endpoint, self.bucket, key);

        let data = self.s3_request("GET", &url, None, None).await?;

        if data.is_empty() {
            return Err(StorageError::NotFound(cid.to_string()));
        }

        Ok(data)
    }

    async fn pin(&self, cid: &CID, policy: PinPolicy) -> Result<(), StorageError> {
        let mut pins = self
            .pins
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock error: {e}")))?;
        pins.insert(
            cid.to_string(),
            PinnedItem {
                cid: *cid,
                policy,
                pinned_at: chrono::Utc::now(),
            },
        );
        Ok(())
    }

    async fn unpin(&self, cid: &CID) -> Result<(), StorageError> {
        let mut pins = self
            .pins
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock error: {e}")))?;
        pins.remove(&cid.to_string());
        Ok(())
    }

    async fn ls_pins(&self) -> Result<Vec<PinnedItem>, StorageError> {
        let pins = self
            .pins
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock error: {e}")))?;
        Ok(pins.values().cloned().collect())
    }

    async fn delete(&self, cid: &CID) -> Result<(), StorageError> {
        let key = Self::key_for(cid);
        let url = format!("{}/{}/{}", self.endpoint, self.bucket, key);

        self.s3_request("DELETE", &url, None, None).await?;
        let _ = self.unpin(cid).await;

        warn!(%cid, "deleted from S3");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_for() {
        let cid: CID = "bafkqaaa".parse().unwrap();
        assert_eq!(S3Backend::key_for(&cid), "bafkqaaa");
    }

    #[test]
    fn test_s3_backend_new() {
        let backend = S3Backend::new(
            "http://localhost:9000".into(),
            "flovenet".into(),
            "us-east-1".into(),
            "minioadmin".into(),
            "minioadmin".into(),
        );
        assert_eq!(backend.bucket, "flovenet");
    }
}

use std::collections::HashMap;

use storage::{
    error::StorageError, PinPolicy, PinnedItem, Result, StorageBackend, StorageOpts, CID,
};
use tracing::{info, warn};

pub struct IpfsBackend {
    api_url: String,
}

impl IpfsBackend {
    pub fn new(api_url: String) -> Self {
        Self { api_url }
    }

    fn api(&self, path: &str) -> String {
        format!("{}/api/v0{}", self.api_url, path)
    }
}

fn parse_add_response(body: &[u8]) -> Result<String> {
    let value: HashMap<String, serde_json::Value> =
        serde_json::from_slice(body).map_err(|e| StorageError::Backend(e.to_string()))?;
    value
        .get("Hash")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| StorageError::Backend("no Hash in response".into()))
}

fn parse_pin_ls_response(body: &[u8]) -> Result<Vec<PinnedItem>> {
    let value: HashMap<String, serde_json::Value> =
        serde_json::from_slice(body).map_err(|e| StorageError::Backend(e.to_string()))?;
    let keys = value
        .get("Keys")
        .and_then(|v| v.as_object())
        .ok_or_else(|| StorageError::Backend("no Keys in response".into()))?;

    let mut items = Vec::new();
    for (cid_str, _) in keys {
        if let Ok(cid) = cid_str.parse::<CID>() {
            items.push(PinnedItem {
                cid,
                policy: PinPolicy::Permanent,
                pinned_at: chrono::Utc::now(),
            });
        }
    }
    Ok(items)
}

#[async_trait::async_trait]
impl StorageBackend for IpfsBackend {
    async fn add(&self, bytes: &[u8], _opts: StorageOpts) -> Result<CID> {
        let url = self.api("/add");
        let body = bytes.to_vec();

        let cid_str = tokio::task::spawn_blocking(move || -> Result<String> {
            let resp = attohttpc::post(&url)
                .header("Content-Type", "application/octet-stream")
                .bytes(body)
                .send()
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            if !resp.is_success() {
                return Err(StorageError::Backend(format!(
                    "IPFS add failed: {}",
                    resp.status()
                )));
            }

            parse_add_response(
                &resp
                    .bytes()
                    .map_err(|e| StorageError::Backend(e.to_string()))?,
            )
        })
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))??;

        let cid = cid_str
            .parse::<CID>()
            .map_err(|e| StorageError::Backend(format!("invalid CID: {e}")))?;

        info!(cid = %cid, "stored on IPFS");
        Ok(cid)
    }

    async fn get(&self, cid: &CID) -> Result<Vec<u8>> {
        let cid_str = cid.to_string();
        let url = self.api(&format!("/cat?arg={cid_str}"));

        let data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let resp = attohttpc::post(&url)
                .send()
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            if !resp.is_success() {
                return Err(StorageError::NotFound(cid_str));
            }

            resp.bytes()
                .map_err(|e| StorageError::Backend(e.to_string()))
        })
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))??;

        Ok(data)
    }

    async fn pin(&self, cid: &CID, _policy: PinPolicy) -> Result<()> {
        let cid_str = cid.to_string();
        let url = self.api(&format!("/pin/add?arg={cid_str}"));

        tokio::task::spawn_blocking(move || -> Result<()> {
            let resp = attohttpc::post(&url)
                .send()
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            if !resp.is_success() {
                return Err(StorageError::Backend(format!(
                    "IPFS pin add failed: {}",
                    resp.status()
                )));
            }

            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))?
    }

    async fn unpin(&self, cid: &CID) -> Result<()> {
        let cid_str = cid.to_string();
        let url = self.api(&format!("/pin/rm?arg={cid_str}"));

        tokio::task::spawn_blocking(move || -> Result<()> {
            let resp = attohttpc::post(&url)
                .send()
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            if !resp.is_success() {
                warn!("IPFS pin rm returned {}", resp.status());
            }

            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))?
    }

    async fn ls_pins(&self) -> Result<Vec<PinnedItem>> {
        let url = self.api("/pin/ls");

        let pins = tokio::task::spawn_blocking(move || -> Result<Vec<PinnedItem>> {
            let resp = attohttpc::post(&url)
                .send()
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            if !resp.is_success() {
                return Err(StorageError::Backend(format!(
                    "IPFS pin ls failed: {}",
                    resp.status()
                )));
            }

            let body = resp
                .bytes()
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            parse_pin_ls_response(&body)
        })
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))??;

        Ok(pins)
    }

    async fn delete(&self, cid: &CID) -> Result<()> {
        let cid_str = cid.to_string();
        let url = self.api(&format!("/block/rm?arg={cid_str}"));

        tokio::task::spawn_blocking(move || -> Result<()> {
            let resp = attohttpc::post(&url)
                .send()
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            if !resp.is_success() {
                warn!("IPFS block rm returned {}", resp.status());
            }

            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipfs_backend_url() {
        let backend = IpfsBackend::new("http://localhost:5001".into());
        assert_eq!(backend.api("/add"), "http://localhost:5001/api/v0/add");
    }

    #[test]
    fn test_parse_add_response() {
        let body = br#"{"Hash":"QmTest","Bytes":12}"#;
        let hash = parse_add_response(body).unwrap();
        assert_eq!(hash, "QmTest");
    }

    #[test]
    fn test_parse_pin_ls_response_empty() {
        let body = br#"{"Keys":{}}"#;
        let items = parse_pin_ls_response(body).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_parse_pin_ls_response_invalid_cid() {
        // Invalid CID strings should be skipped
        let body = br#"{"Keys":{"not-a-cid":{"Type":"recursive"}}}"#;
        let items = parse_pin_ls_response(body).unwrap();
        assert!(items.is_empty());
    }
}

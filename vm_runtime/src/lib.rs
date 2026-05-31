pub mod error;
pub mod wasmtime_runner;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub image_cid: String,
    pub entrypoint: String,
    pub args: Vec<String>,
    pub max_duration_secs: u64,
    pub slots_required: u32,
    pub gpu_vram_gb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub metrics: RunMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetrics {
    pub cpu_usage_percent: f64,
    pub memory_usage_mb: f64,
    pub duration_secs: f64,
}

#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, manifest: Manifest) -> Result<RunResult>;
}

pub type Result<T> = std::result::Result<T, error::RuntimeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_serde() {
        let m = Manifest {
            image_cid: "test.wasm".into(),
            entrypoint: "_start".into(),
            args: vec!["--verbose".into()],
            max_duration_secs: 30,
            slots_required: 2,
            gpu_vram_gb: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        let decoded: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.image_cid, "test.wasm");
        assert_eq!(decoded.entrypoint, "_start");
        assert_eq!(decoded.args, vec!["--verbose".to_string()]);
        assert_eq!(decoded.max_duration_secs, 30);
        assert_eq!(decoded.slots_required, 2);
    }

    #[test]
    fn test_manifest_with_gpu() {
        let m = Manifest {
            image_cid: "gpu.wasm".into(),
            entrypoint: "_start".into(),
            args: vec![],
            max_duration_secs: 60,
            slots_required: 4,
            gpu_vram_gb: Some(8.0),
        };
        let json = serde_json::to_string(&m).unwrap();
        let decoded: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.gpu_vram_gb, Some(8.0));
    }

    #[test]
    fn test_run_result_serde() {
        let r = RunResult {
            stdout: "hello".into(),
            stderr: "".into(),
            exit_code: 0,
            metrics: RunMetrics {
                cpu_usage_percent: 12.5,
                memory_usage_mb: 64.0,
                duration_secs: 1.5,
            },
        };
        let json = serde_json::to_string(&r).unwrap();
        let decoded: RunResult = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.stdout, "hello");
        assert_eq!(decoded.exit_code, 0);
        assert!((decoded.metrics.cpu_usage_percent - 12.5).abs() < 1e-6);
        assert!((decoded.metrics.duration_secs - 1.5).abs() < 1e-6);
    }

    #[test]
    fn test_manifest_default_max_duration() {
        let m = Manifest {
            image_cid: "img".into(),
            entrypoint: "run".into(),
            args: vec![],
            max_duration_secs: 3600,
            slots_required: 1,
            gpu_vram_gb: None,
        };
        assert_eq!(m.max_duration_secs, 3600);
        assert_eq!(m.slots_required, 1);
    }
}

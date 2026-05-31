use libp2p::request_response::{self, json, ProtocolSupport};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

pub const JOB_PROTOCOL: &str = "/flovenet/job/1.1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOffer {
    pub job_id: String,
    pub manifest_cid: String,
    pub slots_required: u32,
    pub max_duration_secs: u64,
    pub reward: Option<u64>,
    pub gpu_vram_gb: Option<f64>,
    pub gpu_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResponse {
    pub job_id: String,
    pub accepted: bool,
    pub reason: Option<String>,
    pub result_cid: Option<String>,
}

pub type JobBehaviour = json::Behaviour<JobOffer, JobResponse>;
pub type JobEvent = request_response::Event<JobOffer, JobResponse>;
pub type JobMessage = request_response::Message<JobOffer, JobResponse>;

pub fn create_job_behaviour() -> JobBehaviour {
    json::Behaviour::new(
        [(StreamProtocol::new(JOB_PROTOCOL), ProtocolSupport::Full)],
        request_response::Config::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_offer_serde() {
        let offer = JobOffer {
            job_id: "test-1".into(),
            manifest_cid: "QmTest123".into(),
            slots_required: 2,
            max_duration_secs: 300,
            reward: None,
            gpu_vram_gb: None,
            gpu_required: false,
        };
        let json = serde_json::to_string(&offer).unwrap();
        let deserialized: JobOffer = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.job_id, "test-1");
    }

    #[test]
    fn test_job_offer_with_gpu() {
        let offer = JobOffer {
            job_id: "gpu-job".into(),
            manifest_cid: "QmGpu123".into(),
            slots_required: 4,
            max_duration_secs: 600,
            reward: Some(100),
            gpu_vram_gb: Some(8.0),
            gpu_required: true,
        };
        let json = serde_json::to_string(&offer).unwrap();
        let deserialized: JobOffer = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.gpu_vram_gb, Some(8.0));
        assert!(deserialized.gpu_required);
    }

    #[test]
    fn test_job_response_serde() {
        let resp = JobResponse {
            job_id: "test-1".into(),
            accepted: true,
            reason: None,
            result_cid: Some("QmResult".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: JobResponse = serde_json::from_str(&json).unwrap();
        assert!(deserialized.accepted);
    }

    #[test]
    fn test_protocol_string() {
        assert_eq!(JOB_PROTOCOL, "/flovenet/job/1.1.0");
    }
}

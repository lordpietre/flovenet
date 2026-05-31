use reputation_engine::ReputationState;
use resource_manager::{NodeDescriptor, NodeRole};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotRequirement {
    pub cpu_cores: u32,
    pub ram_gb: f64,
    pub disk_gb: f64,
    pub gpu_vram_gb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub target_peer: String,
    pub slot_count: u32,
    pub estimated_duration_secs: u64,
}

#[derive(Debug)]
pub enum MatchResult {
    Accepted { slots_used: u32, reason: String },
    Rejected { reason: String },
}

/// Candidate ranking: score + bonuses for matching decisions.
#[derive(Debug)]
pub struct CandidateRank {
    pub peer_id: String,
    pub resources_ok: bool,
    pub reputation_score: f64,
    pub composite_rank: f64,
}

pub struct LocalScheduler {
    reputation: ReputationState,
}

impl LocalScheduler {
    pub fn new() -> Self {
        Self {
            reputation: ReputationState::new(),
        }
    }

    /// Set the reputation state for ranking.
    pub fn with_reputation(mut self, reputation: ReputationState) -> Self {
        self.reputation = reputation;
        self
    }

    /// Update the internal reputation state.
    pub fn set_reputation(&mut self, reputation: ReputationState) {
        self.reputation = reputation;
    }

    /// Merge incoming reputation state.
    pub fn merge_reputation(&mut self, other: &ReputationState) {
        self.reputation.merge(other);
    }

    /// Get a reference to the reputation state.
    pub fn reputation(&self) -> &ReputationState {
        &self.reputation
    }

    pub fn can_accept(
        &self,
        descriptor: &NodeDescriptor,
        requirement: &SlotRequirement,
        role: &NodeRole,
    ) -> MatchResult {
        if descriptor.available_slots < requirement.cpu_cores {
            return MatchResult::Rejected {
                reason: format!(
                    "insufficient slots: have {}, need {}",
                    descriptor.available_slots, requirement.cpu_cores
                ),
            };
        }

        if descriptor.resources.ram_available_gb < requirement.ram_gb {
            return MatchResult::Rejected {
                reason: format!(
                    "insufficient RAM: have {:.1}GB, need {:.1}GB",
                    descriptor.resources.ram_available_gb, requirement.ram_gb
                ),
            };
        }

        if descriptor.resources.disk_available_gb < requirement.disk_gb {
            return MatchResult::Rejected {
                reason: format!(
                    "insufficient disk: have {:.1}GB, need {:.1}GB",
                    descriptor.resources.disk_available_gb, requirement.disk_gb
                ),
            };
        }

        if let Some(gpu_needed) = requirement.gpu_vram_gb {
            match descriptor.resources.gpu_vram_gb {
                Some(gpu_have) if gpu_have >= gpu_needed => {}
                _ => {
                    return MatchResult::Rejected {
                        reason: format!("insufficient GPU VRAM: need {:.1}GB", gpu_needed),
                    }
                }
            }
        }

        if !descriptor.roles.contains(role) {
            return MatchResult::Rejected {
                reason: format!(
                    "role {:?} not supported, available: {:?}",
                    role, descriptor.roles
                ),
            };
        }

        MatchResult::Accepted {
            slots_used: requirement.cpu_cores,
            reason: "resource check passed".into(),
        }
    }

    /// Rank a candidate node for a job, combining resource fit + reputation.
    /// Returns a composite rank — higher is better.
    pub fn rank_candidate(
        &self,
        descriptor: &NodeDescriptor,
        requirement: &SlotRequirement,
        role: &NodeRole,
    ) -> Option<CandidateRank> {
        let resources_ok = match self.can_accept(descriptor, requirement, role) {
            MatchResult::Accepted { .. } => true,
            MatchResult::Rejected { .. } => false,
        };

        let rep = self.reputation.get_score(&descriptor.peer_id);
        let reputation_score = rep.map(|s| s.score).unwrap_or(100.0);

        // Composite rank: resource availability weighted by reputation
        // If resources are insufficient, the rank is still computed but marked
        let resource_factor = if resources_ok {
            1.0
        } else {
            0.0 // will never be selected
        };

        let rep_factor = if reputation_score > 0.0 {
            (reputation_score / 100.0).min(10.0) // cap at 10x baseline
        } else {
            0.1 // very low reputation
        };

        let composite = resource_factor * rep_factor;

        Some(CandidateRank {
            peer_id: descriptor.peer_id.clone(),
            resources_ok,
            reputation_score,
            composite_rank: composite,
        })
    }

    /// Rank multiple candidates and return sorted list (best first).
    pub fn rank_candidates(
        &self,
        descriptors: &[NodeDescriptor],
        requirement: &SlotRequirement,
        role: &NodeRole,
    ) -> Vec<CandidateRank> {
        let mut ranks: Vec<CandidateRank> = descriptors
            .iter()
            .filter_map(|d| self.rank_candidate(d, requirement, role))
            .collect();
        ranks.sort_by(|a, b| {
            b.composite_rank
                .partial_cmp(&a.composite_rank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranks
    }
}

impl Default for LocalScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resource_manager::NodeResources;

    fn test_descriptor() -> NodeDescriptor {
        NodeDescriptor {
            peer_id: "test-peer".into(),
            roles: vec![NodeRole::Compute],
            resources: NodeResources {
                cpu_cores: 8,
                cpu_freq_mhz: 2400.0,
                ram_total_gb: 32.0,
                ram_available_gb: 16.0,
                disk_total_gb: 500.0,
                disk_available_gb: 200.0,
                gpu_vram_gb: None,
                gpu_model: None,
                uptime_secs: 3600,
                platform: resource_manager::Platform::detect(),
            },
            region: "us-east".into(),
            api_url: None,
            total_slots: 8,
            available_slots: 4,
        }
    }

    #[test]
    fn test_accepts_valid_job() {
        let scheduler = LocalScheduler::new();
        let desc = test_descriptor();
        let req = SlotRequirement {
            cpu_cores: 2,
            ram_gb: 4.0,
            disk_gb: 10.0,
            gpu_vram_gb: None,
        };
        match scheduler.can_accept(&desc, &req, &NodeRole::Compute) {
            MatchResult::Accepted { slots_used, .. } => assert_eq!(slots_used, 2),
            MatchResult::Rejected { reason } => panic!("should accept: {reason}"),
        }
    }

    #[test]
    fn test_rejects_insufficient_ram() {
        let scheduler = LocalScheduler::new();
        let desc = test_descriptor();
        let req = SlotRequirement {
            cpu_cores: 1,
            ram_gb: 999.0,
            disk_gb: 1.0,
            gpu_vram_gb: None,
        };
        match scheduler.can_accept(&desc, &req, &NodeRole::Compute) {
            MatchResult::Accepted { .. } => panic!("should reject"),
            MatchResult::Rejected { reason } => {
                assert!(reason.contains("RAM"));
            }
        }
    }

    #[test]
    fn test_rejects_wrong_role() {
        let scheduler = LocalScheduler::new();
        let desc = test_descriptor();
        let req = SlotRequirement {
            cpu_cores: 1,
            ram_gb: 1.0,
            disk_gb: 1.0,
            gpu_vram_gb: None,
        };
        match scheduler.can_accept(&desc, &req, &NodeRole::Storage) {
            MatchResult::Accepted { .. } => panic!("should reject"),
            MatchResult::Rejected { reason } => {
                assert!(reason.contains("role"));
            }
        }
    }

    #[test]
    fn test_rank_prefers_higher_reputation() {
        let mut reputation = ReputationState::new();
        reputation.apply_events(&[
            reputation_engine::ReputationEvent {
                peer_id: "alice".into(),
                timestamp: chrono::Utc::now(),
                kind: reputation_engine::EventKind::Contribution {
                    hours: 100.0,
                    uptime_pct: 99.0,
                },
            },
            reputation_engine::ReputationEvent {
                peer_id: "bob".into(),
                timestamp: chrono::Utc::now(),
                kind: reputation_engine::EventKind::Contribution {
                    hours: 10.0,
                    uptime_pct: 50.0,
                },
            },
        ]);
        reputation.recompute_all();
        let scheduler = LocalScheduler::new().with_reputation(reputation);

        let req = SlotRequirement {
            cpu_cores: 1,
            ram_gb: 1.0,
            disk_gb: 1.0,
            gpu_vram_gb: None,
        };

        let alice_desc = NodeDescriptor {
            peer_id: "alice".into(),
            roles: vec![NodeRole::Compute],
            resources: NodeResources {
                cpu_cores: 4,
                cpu_freq_mhz: 2400.0,
                ram_total_gb: 16.0,
                ram_available_gb: 8.0,
                disk_total_gb: 100.0,
                disk_available_gb: 50.0,
                gpu_vram_gb: None,
                gpu_model: None,
                uptime_secs: 7200,
                platform: resource_manager::Platform::detect(),
            },
            region: "us-east".into(),
            api_url: None,
            total_slots: 4,
            available_slots: 2,
        };

        let bob_desc = NodeDescriptor {
            peer_id: "bob".into(),
            roles: vec![NodeRole::Compute],
            resources: NodeResources {
                cpu_cores: 4,
                cpu_freq_mhz: 2400.0,
                ram_total_gb: 16.0,
                ram_available_gb: 8.0,
                disk_total_gb: 100.0,
                disk_available_gb: 50.0,
                gpu_vram_gb: None,
                gpu_model: None,
                uptime_secs: 3600,
                platform: resource_manager::Platform::detect(),
            },
            region: "us-east".into(),
            api_url: None,
            total_slots: 4,
            available_slots: 3,
        };

        let candidates =
            scheduler.rank_candidates(&[bob_desc, alice_desc], &req, &NodeRole::Compute);
        assert_eq!(candidates.len(), 2);
        // Alice has higher reputation, should be ranked first
        assert_eq!(candidates[0].peer_id, "alice");
        assert!(candidates[0].composite_rank > candidates[1].composite_rank);
    }

    #[test]
    fn test_rejects_insufficient_slots() {
        let scheduler = LocalScheduler::new();
        let desc = test_descriptor();
        let req = SlotRequirement {
            cpu_cores: 999,
            ram_gb: 1.0,
            disk_gb: 1.0,
            gpu_vram_gb: None,
        };
        match scheduler.can_accept(&desc, &req, &NodeRole::Compute) {
            MatchResult::Accepted { .. } => panic!("should reject"),
            MatchResult::Rejected { reason } => assert!(reason.contains("slots")),
        }
    }

    #[test]
    fn test_rejects_insufficient_disk() {
        let scheduler = LocalScheduler::new();
        let desc = test_descriptor();
        let req = SlotRequirement {
            cpu_cores: 1,
            ram_gb: 1.0,
            disk_gb: 9999.0,
            gpu_vram_gb: None,
        };
        match scheduler.can_accept(&desc, &req, &NodeRole::Compute) {
            MatchResult::Accepted { .. } => panic!("should reject"),
            MatchResult::Rejected { reason } => assert!(reason.contains("disk")),
        }
    }

    #[test]
    fn test_rejects_missing_gpu() {
        let scheduler = LocalScheduler::new();
        let mut desc = test_descriptor();
        desc.resources.gpu_vram_gb = None; // no GPU
        let req = SlotRequirement {
            cpu_cores: 1,
            ram_gb: 1.0,
            disk_gb: 1.0,
            gpu_vram_gb: Some(8.0),
        };
        match scheduler.can_accept(&desc, &req, &NodeRole::Compute) {
            MatchResult::Accepted { .. } => panic!("should reject"),
            MatchResult::Rejected { reason } => assert!(reason.contains("GPU")),
        }
    }

    #[test]
    fn test_accepts_with_sufficient_gpu() {
        let scheduler = LocalScheduler::new();
        let mut desc = test_descriptor();
        desc.resources.gpu_vram_gb = Some(24.0);
        let req = SlotRequirement {
            cpu_cores: 1,
            ram_gb: 1.0,
            disk_gb: 1.0,
            gpu_vram_gb: Some(8.0),
        };
        assert!(matches!(
            scheduler.can_accept(&desc, &req, &NodeRole::Compute),
            MatchResult::Accepted { .. }
        ));
    }

    #[test]
    fn test_rank_candidates_empty_list() {
        let scheduler = LocalScheduler::new();
        let req = SlotRequirement {
            cpu_cores: 1,
            ram_gb: 1.0,
            disk_gb: 1.0,
            gpu_vram_gb: None,
        };
        let candidates = scheduler.rank_candidates(&[], &req, &NodeRole::Compute);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_rank_candidate_not_resources_ok() {
        let scheduler = LocalScheduler::new();
        let req = SlotRequirement {
            cpu_cores: 999,
            ram_gb: 1.0,
            disk_gb: 1.0,
            gpu_vram_gb: None,
        };
        let desc = test_descriptor();
        let rank = scheduler
            .rank_candidate(&desc, &req, &NodeRole::Compute)
            .unwrap();
        assert!(!rank.resources_ok);
        assert_eq!(rank.composite_rank, 0.0);
    }

    #[test]
    fn test_set_and_merge_reputation() {
        let mut rep = reputation_engine::ReputationState::new();
        rep.apply_events(&[reputation_engine::ReputationEvent {
            peer_id: "node1".into(),
            timestamp: chrono::Utc::now(),
            kind: reputation_engine::EventKind::Contribution {
                hours: 10.0,
                uptime_pct: 99.0,
            },
        }]);
        rep.recompute_all();

        let mut scheduler = LocalScheduler::new();
        scheduler.set_reputation(rep.clone());
        assert!(scheduler.reputation().get_score("node1").is_some());

        let mut rep2 = reputation_engine::ReputationState::new();
        rep2.apply_events(&[reputation_engine::ReputationEvent {
            peer_id: "node2".into(),
            timestamp: chrono::Utc::now(),
            kind: reputation_engine::EventKind::Contribution {
                hours: 5.0,
                uptime_pct: 80.0,
            },
        }]);
        rep2.recompute_all();

        scheduler.merge_reputation(&rep2);
        assert_eq!(scheduler.reputation().peer_count(), 2);
    }

    #[test]
    fn test_slot_requirement_serde() {
        let req = SlotRequirement {
            cpu_cores: 4,
            ram_gb: 8.0,
            disk_gb: 50.0,
            gpu_vram_gb: Some(16.0),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: SlotRequirement = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.gpu_vram_gb, Some(16.0));
    }

    #[test]
    fn test_gpu_job_end_to_end() {
        let scheduler = LocalScheduler::new();
        let mut desc = test_descriptor();
        desc.resources.gpu_vram_gb = Some(24.0);
        desc.resources.gpu_model = Some("RTX 4090".into());
        desc.roles = vec![NodeRole::Ai];

        let req = SlotRequirement {
            cpu_cores: 2,
            ram_gb: 4.0,
            disk_gb: 10.0,
            gpu_vram_gb: Some(8.0),
        };

        match scheduler.can_accept(&desc, &req, &NodeRole::Ai) {
            MatchResult::Accepted { .. } => {}
            MatchResult::Rejected { reason } => panic!("should accept GPU job: {reason}"),
        }
    }

    #[test]
    fn test_gpu_job_rejected_without_gpu() {
        let scheduler = LocalScheduler::new();
        let mut desc = test_descriptor();
        desc.resources.gpu_vram_gb = None;
        desc.roles = vec![NodeRole::Ai];

        let req = SlotRequirement {
            cpu_cores: 2,
            ram_gb: 4.0,
            disk_gb: 10.0,
            gpu_vram_gb: Some(8.0),
        };

        match scheduler.can_accept(&desc, &req, &NodeRole::Ai) {
            MatchResult::Accepted { .. } => panic!("should reject GPU job without GPU"),
            MatchResult::Rejected { reason } => assert!(reason.contains("GPU")),
        }
    }

    #[test]
    fn test_gpu_job_rejected_insufficient_vram() {
        let scheduler = LocalScheduler::new();
        let mut desc = test_descriptor();
        desc.resources.gpu_vram_gb = Some(4.0);
        desc.roles = vec![NodeRole::Ai];

        let req = SlotRequirement {
            cpu_cores: 2,
            ram_gb: 4.0,
            disk_gb: 10.0,
            gpu_vram_gb: Some(8.0),
        };

        match scheduler.can_accept(&desc, &req, &NodeRole::Ai) {
            MatchResult::Accepted { .. } => panic!("should reject GPU job with insufficient VRAM"),
            MatchResult::Rejected { reason } => assert!(reason.contains("GPU")),
        }
    }
}

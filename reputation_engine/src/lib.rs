use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Reputation score for a peer, computed from events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationScore {
    pub peer_id: String,
    /// Composite score: contribution_hours * uptime_pct * success_rate * diversity_factor
    pub score: f64,
    /// Total hours of compute donated (as provider)
    pub contribution_hours: f64,
    /// Total hours of compute consumed (as requester)
    pub consumption_hours: f64,
    /// Net contribution (contribution - consumption)
    pub net_contribution: f64,
    /// Uptime percentage over observed window (0.0 – 100.0)
    pub uptime_pct: f64,
    /// Fraction of completed jobs that succeeded (0.0 – 1.0)
    pub success_rate: f64,
    /// Number of distinct peers interacted with
    pub peer_diversity: u32,
    /// Bonus points from serving popular content or verification
    pub bonus: f64,
    /// When this score was last updated
    pub updated_at: DateTime<Utc>,
}

/// A reputation event — the atomic unit of the CRDT.
/// Each event is signed by its origin peer and propagated via Gossipsub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationEvent {
    pub peer_id: String,
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventKind {
    /// Donated `hours` of compute time with given uptime
    Contribution { hours: f64, uptime_pct: f64 },
    /// Consumed `hours` of compute time
    Consumption { hours: f64 },
    /// A job completed successfully
    JobSuccess,
    /// A job failed
    JobFailure,
    /// Uptime measurement update
    UptimeUpdate { uptime_pct: f64 },
    /// Bonus for serving popular content
    BonusContent { amount: f64 },
    /// Bonus for successful verification of another peer's work
    BonusVerification { amount: f64 },
}

/// Reputation state — a CRDT map of peer_id → ReputationScore with LWW merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationState {
    /// Map of peer_id to their latest known score + timestamp
    pub peers: HashMap<String, ScoredPeer>,
}

/// A scored peer with timestamp for LWW conflict resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredPeer {
    pub score: ReputationScore,
    pub updated_at: DateTime<Utc>,
}

impl Default for ReputationState {
    fn default() -> Self {
        Self::new()
    }
}

impl ReputationState {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Apply a single event to this node's own state, updating the score.
    pub fn apply_event(&mut self, event: &ReputationEvent) {
        let entry = self
            .peers
            .entry(event.peer_id.clone())
            .or_insert_with(|| ScoredPeer {
                score: ReputationScore::new(&event.peer_id),
                updated_at: event.timestamp,
            });

        // Only apply if this event is newer than our current state
        if event.timestamp < entry.updated_at {
            return;
        }

        let score = &mut entry.score;
        match &event.kind {
            EventKind::Contribution { hours, uptime_pct } => {
                score.contribution_hours += hours;
                // Weighted average for uptime
                let total = score.contribution_hours;
                if total > 0.0 {
                    let old_weight = (total - hours) / total;
                    let new_weight = hours / total;
                    score.uptime_pct = score.uptime_pct * old_weight + uptime_pct * new_weight;
                } else {
                    score.uptime_pct = *uptime_pct;
                }
                score.peer_diversity = score.peer_diversity.saturating_add(1);
            }
            EventKind::Consumption { hours } => {
                score.consumption_hours += hours;
                score.peer_diversity = score.peer_diversity.saturating_add(1);
            }
            EventKind::JobSuccess => {
                // Tracked via success_rate below
            }
            EventKind::JobFailure => {}
            EventKind::UptimeUpdate { uptime_pct } => {
                score.uptime_pct = *uptime_pct;
            }
            EventKind::BonusContent { amount } => {
                score.bonus += amount;
            }
            EventKind::BonusVerification { amount } => {
                score.bonus += amount;
            }
        }

        // Recompute derived fields
        score.net_contribution = score.contribution_hours - score.consumption_hours;
        entry.updated_at = event.timestamp;
    }

    /// Recompute the composite score for a peer.
    pub fn recompute_score(&mut self, peer_id: &str) {
        if let Some(entry) = self.peers.get_mut(peer_id) {
            entry.score.score = entry.score.compute();
            entry.updated_at = Utc::now();
        }
    }

    /// Recompute scores for all peers.
    pub fn recompute_all(&mut self) {
        let now = Utc::now();
        for entry in self.peers.values_mut() {
            entry.score.score = entry.score.compute();
            entry.updated_at = now;
        }
    }

    /// CRDT merge: take the newer version for each peer_id.
    pub fn merge(&mut self, other: &ReputationState) {
        for (peer_id, other_entry) in &other.peers {
            let current = self.peers.get(peer_id);
            if current.is_none_or(|c| other_entry.updated_at > c.updated_at) {
                self.peers.insert(peer_id.clone(), other_entry.clone());
            }
        }
    }

    /// Get a peer's reputation score.
    pub fn get_score(&self, peer_id: &str) -> Option<&ReputationScore> {
        self.peers.get(peer_id).map(|s| &s.score)
    }

    /// Get sorted leaderboard (highest score first).
    pub fn leaderboard(&self) -> Vec<&ReputationScore> {
        let mut scores: Vec<&ReputationScore> = self.peers.values().map(|s| &s.score).collect();
        scores.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scores
    }

    /// Top N peers by reputation.
    pub fn top_n(&self, n: usize) -> Vec<&ReputationScore> {
        self.leaderboard().into_iter().take(n).collect()
    }

    /// Total number of tracked peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Process a batch of events (e.g., from a gossip message).
    pub fn apply_events(&mut self, events: &[ReputationEvent]) {
        for event in events {
            self.apply_event(event);
        }
        self.recompute_all();
    }
}

impl ReputationScore {
    pub fn new(peer_id: &str) -> Self {
        Self {
            peer_id: peer_id.to_string(),
            score: 100.0, // starting score
            contribution_hours: 0.0,
            consumption_hours: 0.0,
            net_contribution: 0.0,
            uptime_pct: 0.0,
            success_rate: 1.0, // start perfect
            peer_diversity: 0,
            bonus: 0.0,
            updated_at: Utc::now(),
        }
    }

    /// Compute composite score: base + net_contribution_points × multipliers + bonus
    fn compute(&self) -> f64 {
        let base = if self.contribution_hours == 0.0 && self.consumption_hours == 0.0 {
            100.0 // baseline for new peers
        } else {
            // 10 points per hour of net contribution
            let hourly_rate = 10.0;
            let net_points = self.net_contribution.max(0.0) * hourly_rate;

            // Uptime multiplier: 0.5 – 1.0
            let uptime_mult = 0.5 + (self.uptime_pct / 100.0) * 0.5;
            // Success rate multiplier: 0.5 – 1.0
            let success_mult = 0.5 + self.success_rate * 0.5;
            // Diversity multiplier: 1.0 – 2.0 (capped at 50 unique peers)
            let diversity_mult = 1.0 + (self.peer_diversity as f64).min(50.0) * 0.02;

            50.0 + net_points * uptime_mult * success_mult * diversity_mult
        };
        (base + self.bonus).max(0.0)
    }

    /// Record a job completion (success or failure).
    pub fn record_job_outcome(&mut self, success: bool) {
        // Simplified: track approximate success rate based on count
        // A real implementation would track actual counts
        if success {
            self.success_rate = (self.success_rate * 0.95) + (1.0 * 0.05);
        } else {
            self.success_rate *= 0.95;
        }
        self.score = self.compute();
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(peer_id: &str, kind: EventKind) -> ReputationEvent {
        ReputationEvent {
            peer_id: peer_id.to_string(),
            timestamp: Utc::now(),
            kind,
        }
    }

    #[test]
    fn test_new_peer_baseline() {
        let score = ReputationScore::new("peer1");
        assert_eq!(score.score, 100.0);
    }

    #[test]
    fn test_contribution_increases_score() {
        let mut state = ReputationState::new();
        state.apply_events(&[
            make_event(
                "peer1",
                EventKind::Contribution {
                    hours: 10.0,
                    uptime_pct: 99.0,
                },
            ),
            make_event(
                "peer1",
                EventKind::Contribution {
                    hours: 5.0,
                    uptime_pct: 95.0,
                },
            ),
        ]);
        state.recompute_score("peer1");
        let score = state.get_score("peer1").unwrap();
        assert!(score.score > 150.0);
        assert!((score.contribution_hours - 15.0).abs() < 0.001);
        assert!((score.uptime_pct - 97.67).abs() < 0.1);
    }

    #[test]
    fn test_consumption_reduces_net_contribution() {
        let mut state = ReputationState::new();
        state.apply_events(&[
            make_event(
                "peer1",
                EventKind::Contribution {
                    hours: 20.0,
                    uptime_pct: 99.0,
                },
            ),
            make_event("peer1", EventKind::Consumption { hours: 5.0 }),
        ]);
        state.recompute_score("peer1");
        let score = state.get_score("peer1").unwrap();
        assert!((score.net_contribution - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_leaderboard_ordering() {
        let mut state = ReputationState::new();
        state.apply_events(&[
            make_event(
                "alice",
                EventKind::Contribution {
                    hours: 50.0,
                    uptime_pct: 99.0,
                },
            ),
            make_event(
                "bob",
                EventKind::Contribution {
                    hours: 10.0,
                    uptime_pct: 50.0,
                },
            ),
            make_event(
                "carol",
                EventKind::Contribution {
                    hours: 30.0,
                    uptime_pct: 90.0,
                },
            ),
        ]);
        state.recompute_all();
        let board = state.leaderboard();
        assert_eq!(board[0].peer_id, "alice");
        assert_eq!(board[1].peer_id, "carol");
        assert_eq!(board[2].peer_id, "bob");
    }

    #[test]
    fn test_crdt_merge() {
        let mut state_a = ReputationState::new();
        state_a.apply_events(&[make_event(
            "peer1",
            EventKind::Contribution {
                hours: 10.0,
                uptime_pct: 99.0,
            },
        )]);
        state_a.recompute_all();

        let mut state_b = ReputationState::new();
        state_b.apply_events(&[make_event(
            "peer2",
            EventKind::Contribution {
                hours: 5.0,
                uptime_pct: 80.0,
            },
        )]);
        state_b.recompute_all();

        state_a.merge(&state_b);
        assert_eq!(state_a.peer_count(), 2);
        assert!(state_a.get_score("peer1").is_some());
        assert!(state_a.get_score("peer2").is_some());
    }

    #[test]
    fn test_bonus_adds_to_score() {
        let mut state = ReputationState::new();
        state.apply_events(&[
            make_event(
                "peer1",
                EventKind::Contribution {
                    hours: 10.0,
                    uptime_pct: 99.0,
                },
            ),
            make_event("peer1", EventKind::BonusContent { amount: 50.0 }),
        ]);
        state.recompute_score("peer1");
        let score = state.get_score("peer1").unwrap();
        assert!((score.bonus - 50.0).abs() < 0.001);
        assert!(score.score > 100.0);
    }

    #[test]
    fn test_old_event_is_ignored() {
        let mut state = ReputationState::new();
        let old_event = ReputationEvent {
            peer_id: "peer1".to_string(),
            timestamp: DateTime::from_timestamp(1000, 0).unwrap(),
            kind: EventKind::Contribution {
                hours: 1.0,
                uptime_pct: 10.0,
            },
        };
        let new_event = make_event(
            "peer1",
            EventKind::Contribution {
                hours: 100.0,
                uptime_pct: 99.0,
            },
        );

        state.apply_event(&new_event);
        state.apply_event(&old_event); // should be ignored
        state.recompute_score("peer1");
        let score = state.get_score("peer1").unwrap();
        assert!((score.contribution_hours - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_top_n() {
        let mut state = ReputationState::new();
        for i in 0..5 {
            state.apply_events(&[make_event(
                &format!("peer{i}"),
                EventKind::Contribution {
                    hours: (i as f64 + 1.0) * 10.0,
                    uptime_pct: 99.0,
                },
            )]);
        }
        state.recompute_all();
        let top = state.top_n(3);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].peer_id, "peer4");
    }

    #[test]
    fn test_job_outcome_updates_success_rate() {
        let mut score = ReputationScore::new("peer1");
        let initial = score.success_rate;
        score.record_job_outcome(false);
        assert!(score.success_rate < initial);
        score.record_job_outcome(true);
        assert!(score.success_rate > 0.0);
    }

    #[test]
    fn test_empty_state() {
        let state = ReputationState::new();
        assert_eq!(state.peer_count(), 0);
        assert!(state.leaderboard().is_empty());
        assert!(state.top_n(5).is_empty());
        assert!(state.get_score("nonexistent").is_none());
    }

    #[test]
    fn test_baseline_with_no_events() {
        let state = ReputationState::new();
        // New peer with no events gets the default score
        let score = ReputationScore::new("newbie");
        assert_eq!(score.score, 100.0);
        // Actually add to state and check
        let score_from_state = state.get_score("newbie");
        assert!(score_from_state.is_none());
    }

    #[test]
    fn test_multiple_bonus_events_accumulate() {
        let mut state = ReputationState::new();
        state.apply_events(&[
            make_event("peer1", EventKind::BonusContent { amount: 25.0 }),
            make_event("peer1", EventKind::BonusVerification { amount: 15.0 }),
        ]);
        state.recompute_score("peer1");
        let score = state.get_score("peer1").unwrap();
        assert!((score.bonus - 40.0).abs() < 0.001);
    }

    #[test]
    fn test_consumption_exceeds_contribution() {
        let mut state = ReputationState::new();
        state.apply_events(&[
            make_event(
                "peer1",
                EventKind::Contribution {
                    hours: 5.0,
                    uptime_pct: 99.0,
                },
            ),
            make_event("peer1", EventKind::Consumption { hours: 10.0 }),
        ]);
        state.recompute_score("peer1");
        let score = state.get_score("peer1").unwrap();
        // Net contribution should be negative
        assert!(score.net_contribution < 0.0);
        // Score should be baseline + bonus (no bonus), with 0 net contribution for scoring
        assert!(score.score >= 50.0);
        assert!(score.score < 200.0); // should be near baseline
    }

    #[test]
    fn test_uptime_update_overwrites() {
        let mut state = ReputationState::new();
        state.apply_events(&[make_event(
            "peer1",
            EventKind::Contribution {
                hours: 10.0,
                uptime_pct: 50.0,
            },
        )]);
        state.apply_events(&[make_event(
            "peer1",
            EventKind::UptimeUpdate { uptime_pct: 99.0 },
        )]);
        state.recompute_score("peer1");
        let score = state.get_score("peer1").unwrap();
        assert!((score.uptime_pct - 99.0).abs() < 0.001);
    }

    #[test]
    fn test_score_never_negative() {
        let mut score = ReputationScore::new("peer1");
        score.bonus = -9999.0; // extreme negative bonus
        let computed = score.compute();
        assert!(computed >= 0.0);
    }

    #[test]
    fn test_merge_keeps_newer() {
        let mut state_old = ReputationState::new();
        let old_event = ReputationEvent {
            peer_id: "peer1".to_string(),
            timestamp: DateTime::from_timestamp(1000, 0).unwrap(),
            kind: EventKind::Contribution {
                hours: 1.0,
                uptime_pct: 10.0,
            },
        };
        state_old.apply_event(&old_event);
        state_old.recompute_all();

        let mut state_new = ReputationState::new();
        state_new.apply_events(&[make_event(
            "peer1",
            EventKind::Contribution {
                hours: 99.0,
                uptime_pct: 99.0,
            },
        )]);
        state_new.recompute_all();

        // Old should have its data, new should have newer data
        state_old.merge(&state_new);
        let score = state_old.get_score("peer1").unwrap();
        assert!((score.contribution_hours - 99.0).abs() < 0.001);
    }
}

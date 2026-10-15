//! Rows of `pull_request_review_guides` and `pull_request_review_guide_runs`.

use chrono::{DateTime, Utc};

/// One pull request on one git host. `repository` is `owner/repo`, stored
/// lowercase so a reference that differs only in case finds the same guide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestKey {
    pub host: String,
    pub repository: String,
    pub number: i64,
}

impl PullRequestKey {
    pub fn new(host: &str, repository: &str, number: u64) -> Self {
        Self {
            host: host.to_ascii_lowercase(),
            repository: repository.to_ascii_lowercase(),
            number: i64::try_from(number).unwrap_or(i64::MAX),
        }
    }
}

/// A stored guide.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredReviewGuide {
    pub head_sha: String,
    /// `{ "stops": [...] }`.
    pub guide: serde_json::Value,
    pub task_id: String,
    pub agent_profile: Option<String>,
    pub model: Option<String>,
    pub generated_at: DateTime<Utc>,
}

/// A run in progress, or the last failed one.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReviewGuideRun {
    pub host: String,
    pub repository: String,
    pub number: i64,
    pub run_id: String,
    pub head_sha: String,
    pub base_sha: String,
    pub task_id: Option<String>,
    /// `running` or `failed`.
    pub status: String,
    pub detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub failed_at: Option<DateTime<Utc>>,
}

impl ReviewGuideRun {
    pub fn is_running(&self) -> bool {
        self.status == RUN_RUNNING
    }
}

pub(crate) const RUN_RUNNING: &str = "running";
pub(crate) const RUN_FAILED: &str = "failed";

/// The outcome of asking to start a run.
#[derive(Debug, Clone)]
pub enum RunClaim {
    /// This call owns a new run, whose Task it must now create and attach.
    Claimed(ReviewGuideRun),
    /// A run for the same head is already going.
    Joined(ReviewGuideRun),
}

//! Guided-review request/response types: the tour of a pull request an agent
//! writes and submits with `moat review-guide submit`.
//!
//! A stop serializes in T3's `PullRequestReviewGuideStop` shape, so a stored
//! guide is served to the client as it was accepted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Response for `GET /api/v1/tasks/{task_id}/review-guide`: the pull request
/// and range the Task was started to write a guide for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReviewGuideRunResponse {
    /// `owner/repo`.
    pub repository: String,
    pub number: u64,
    /// The base branch's tip when the run started; the change is
    /// `baseSha...headSha`.
    pub base_sha: String,
    pub head_sha: String,
}

/// Request body for `POST /api/v1/tasks/{task_id}/review-guide`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SubmitReviewGuideRequest {
    pub stops: Vec<ReviewGuideStop>,
    /// The lines the change touched, read by the CLI from the checkout. The
    /// server holds the stops to it.
    pub change: ReviewGuideChange,
}

/// What a guide is held to: the changed paths and the lines each touched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReviewGuideChange {
    /// Every changed path, in diff order.
    pub files: Vec<String>,
    /// Keyed by the new path, or the old one for a deleted file.
    #[serde(default)]
    pub hunks: BTreeMap<String, ReviewGuideFileHunks>,
}

/// The changed line ranges of one file, on each side of the diff.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewGuideFileHunks {
    #[serde(default)]
    pub new: Vec<ReviewGuideLineRange>,
    #[serde(default)]
    pub old: Vec<ReviewGuideLineRange>,
}

/// An inclusive range of line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewGuideLineRange {
    pub start: u32,
    pub end: u32,
}

/// One stop of the tour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewGuideStop {
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub attention: Option<ReviewGuideAttention>,
    #[serde(default)]
    pub excerpts: Vec<ReviewGuideExcerpt>,
    /// Changed files the tour never excerpts, each listed by one stop.
    #[serde(default)]
    pub files: Vec<ReviewGuideFile>,
    #[serde(default)]
    pub notes: Vec<ReviewGuideNote>,
    #[serde(default)]
    pub questions: Vec<String>,
}

/// The one call a maintainer should make at a stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewGuideAttention {
    pub kind: ReviewGuideAttentionKind,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ReviewGuideAttentionKind {
    Tradeoff,
    Contract,
    Product,
    Risk,
    Accessibility,
}

/// Which side of the diff an excerpt's line numbers count on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ReviewGuideSide {
    New,
    Old,
}

/// Lines worth reading, inside one hunk of the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewGuideExcerpt {
    pub path: String,
    pub side: ReviewGuideSide,
    pub start: u32,
    pub end: u32,
    #[serde(default)]
    pub caption: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewGuideFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// An explanation anchored to a line inside one of its stop's excerpts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewGuideNote {
    pub path: String,
    pub line: u32,
    pub title: String,
    #[serde(default)]
    pub body: String,
}

/// Response for a submitted guide: what was kept once it was held to the
/// change, so the agent can tell a guide that landed from one that was trimmed
/// to nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SubmitReviewGuideResponse {
    pub stops: usize,
    pub excerpts: usize,
    /// Excerpts dropped because no line of theirs is inside the change.
    pub dropped_excerpts: usize,
    /// Notes dropped because their line is outside every kept excerpt.
    pub dropped_notes: usize,
    /// Changed files no stop placed, gathered into a closing stop.
    pub unplaced_files: usize,
}

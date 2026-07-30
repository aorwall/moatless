//! Typed GitHub PR signals persisted on `task_bindings`: the PR's review
//! decision (was it approved?) and its mergeable state (does it conflict?).
//!
//! Both are stored as canonical lowercase strings in dedicated columns (see
//! migration `0076`). These enums give the write/derive sites a typed, spelling-
//! safe view; the DB model keeps the raw `Option<String>` to match the existing
//! `external_status` column pattern.

/// GitHub's coarse mergeability state for a PR (`pull_request.mergeable_state`
/// / the single-PR API's `mergeable_state`). GitHub computes this
/// asynchronously, so a fresh read is often [`MergeableState::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeableState {
    /// No conflicts — mergeable.
    Clean,
    /// Merge conflicts with the base branch.
    Dirty,
    /// Blocked by a required check / review / branch protection.
    Blocked,
    /// The head branch is behind the base and must be updated first.
    Behind,
    /// A non-required check is failing.
    Unstable,
    /// A pre-receive hook is configured.
    HasHooks,
    /// The PR is a draft.
    Draft,
    /// Not yet computed by GitHub — inconclusive.
    Unknown,
}

impl MergeableState {
    /// Parse GitHub's `mergeable_state` string. Unrecognized values map to
    /// [`MergeableState::Unknown`].
    pub fn from_github(raw: &str) -> Self {
        match raw {
            "clean" => Self::Clean,
            "dirty" => Self::Dirty,
            "blocked" => Self::Blocked,
            "behind" => Self::Behind,
            "unstable" => Self::Unstable,
            "has_hooks" => Self::HasHooks,
            "draft" => Self::Draft,
            _ => Self::Unknown,
        }
    }

    /// Canonical string persisted to `external_mergeable_state`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Blocked => "blocked",
            Self::Behind => "behind",
            Self::Unstable => "unstable",
            Self::HasHooks => "has_hooks",
            Self::Draft => "draft",
            Self::Unknown => "unknown",
        }
    }

    /// Whether GitHub has actually resolved the state (not [`Self::Unknown`]).
    /// An unresolved state is never persisted — it would just churn the column.
    pub fn is_settled(self) -> bool {
        self != Self::Unknown
    }

    /// Whether the PR currently has merge conflicts.
    pub fn is_conflicted(self) -> bool {
        self == Self::Dirty
    }
}

/// Latest review verdict recorded for a PR. Mirrors the states GitHub reports
/// on a `pull_request_review` submission, plus GitHub's PR-level
/// `review_required` bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    /// At least one approving review, no outstanding change requests.
    Approved,
    /// A reviewer requested changes.
    ChangesRequested,
    /// A review is required but none has landed yet.
    ReviewRequired,
    /// A review was dismissed.
    Dismissed,
    /// A review was left as a plain comment (no verdict).
    Commented,
}

impl ReviewDecision {
    /// Map the `review.state` of a `pull_request_review` webhook to a decision.
    /// Returns `None` for states we don't persist as a verdict (e.g. `pending`).
    pub fn from_review_state(state: &str) -> Option<Self> {
        match state {
            "approved" => Some(Self::Approved),
            "changes_requested" => Some(Self::ChangesRequested),
            "dismissed" => Some(Self::Dismissed),
            "commented" => Some(Self::Commented),
            _ => None,
        }
    }

    /// Canonical string persisted to `external_review_decision`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes_requested",
            Self::ReviewRequired => "review_required",
            Self::Dismissed => "dismissed",
            Self::Commented => "commented",
        }
    }

    /// Whether this decision reflects a real verdict worth overwriting a prior
    /// decision with. A plain `commented` review must not clobber an existing
    /// `approved` / `changes_requested`, so it is not a verdict.
    pub fn is_verdict(self) -> bool {
        matches!(
            self,
            Self::Approved | Self::ChangesRequested | Self::Dismissed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mergeable_state_roundtrips_known_values() {
        for raw in [
            "clean",
            "dirty",
            "blocked",
            "behind",
            "unstable",
            "has_hooks",
            "draft",
        ] {
            assert_eq!(MergeableState::from_github(raw).as_str(), raw);
        }
    }

    #[test]
    fn mergeable_state_unknown_for_unrecognized() {
        assert_eq!(
            MergeableState::from_github("weird"),
            MergeableState::Unknown
        );
        assert!(!MergeableState::Unknown.is_settled());
        assert!(MergeableState::Clean.is_settled());
        assert!(MergeableState::Dirty.is_conflicted());
        assert!(!MergeableState::Clean.is_conflicted());
    }

    #[test]
    fn review_decision_maps_states() {
        assert_eq!(
            ReviewDecision::from_review_state("approved"),
            Some(ReviewDecision::Approved)
        );
        assert_eq!(
            ReviewDecision::from_review_state("changes_requested"),
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(ReviewDecision::from_review_state("pending"), None);
        assert_eq!(ReviewDecision::Approved.as_str(), "approved");
        assert!(ReviewDecision::Approved.is_verdict());
        assert!(!ReviewDecision::Commented.is_verdict());
    }
}

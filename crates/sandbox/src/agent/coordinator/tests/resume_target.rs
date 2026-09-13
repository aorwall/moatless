//! Which session a replayed or next Turn resumes.
//!
//! Session state is restored to disk during init before Ready, and the harness
//! is fixed for the Sandbox, so a resumed run must target the restored session
//! id for whichever family the pod runs.

use super::*;

#[test]
fn build_resume_prefers_last_session_id_for_claude() {
    // The pod's own last-known Session id wins over any backend-provided id.
    let resume = build_resume(
        "restored-claude-session",
        Some("backend-resume-id"),
        AGENT_TYPE_CLAUDE_CODE,
        false,
    );
    assert_eq!(
        resume,
        Some(ResumeSession::WithId("restored-claude-session".to_string()))
    );
}

#[test]
fn build_resume_prefers_last_session_id_for_codex() {
    let resume = build_resume(
        "restored-codex-rollout",
        Some("backend-resume-id"),
        "codex",
        false,
    );
    assert_eq!(
        resume,
        Some(ResumeSession::WithId("restored-codex-rollout".to_string()))
    );
}

#[test]
fn build_resume_uses_backend_id_on_fresh_pod_for_both_families() {
    // Fresh pod with no local Session: the backend-provided resume id (from the
    // restored run) is honored for both families.
    for family in [AGENT_TYPE_CLAUDE_CODE, "codex"] {
        let resume = build_resume("", Some("backend-resume-id"), family, false);
        assert_eq!(
            resume,
            Some(ResumeSession::WithId("backend-resume-id".to_string())),
            "family {family}"
        );
    }
}

#[test]
fn build_resume_start_fresh_overrides_restored_session_for_both_families() {
    for family in [AGENT_TYPE_CLAUDE_CODE, "codex"] {
        let resume = build_resume("restored-session", Some("backend-resume-id"), family, true);
        assert_eq!(resume, None, "family {family}");
    }
}

#[test]
fn build_resume_falls_back_to_continue_for_claude_only() {
    // No local Session and no backend id: Claude gets `--continue`, Codex (which
    // has no `--continue`) starts fresh.
    assert_eq!(
        build_resume("", None, AGENT_TYPE_CLAUDE_CODE, false),
        Some(ResumeSession::Last)
    );
    assert_eq!(build_resume("", None, "codex", false), None);
}

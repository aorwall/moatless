//! Claude Session discovery rules.
//!
//! Explicit resume requests are checked against the restored transcript store
//! before the CLI starts. This makes a missing archive a stable harness fact
//! instead of depending on which native startup error a Claude CLI release
//! happens to emit.

use std::path::{Path, PathBuf};

use crate::agent::harness::trajectory::ClaudeSessionFormat;
use crate::agent::types::ResumeSession;
use crate::config::Config;

use super::super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};

pub(super) async fn ensure_explicit_resume_is_available(
    config: &Config,
    resume: &Option<ResumeSession>,
    start_fresh: bool,
) -> Result<(), HarnessError> {
    let Some(ResumeSession::WithId(session_id)) = resume else {
        return Ok(());
    };
    if start_fresh {
        return Ok(());
    }

    let transcript = transcript_path(config, session_id).await;
    match tokio::fs::try_exists(&transcript).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(unavailable_session(
            session_id,
            format!(
                "restored Claude transcript is missing: {}",
                transcript.display()
            ),
        )),
        Err(error) => Err(unavailable_session(
            session_id,
            format!(
                "restored Claude transcript is unavailable at {}: {error}",
                transcript.display()
            ),
        )),
    }
}

async fn transcript_path(config: &Config, session_id: &str) -> PathBuf {
    let workspace = Path::new(&config.workspace_dir);
    let resolved = tokio::fs::canonicalize(workspace)
        .await
        .unwrap_or_else(|_| workspace.to_path_buf());
    Path::new(&config.claude_config_dir)
        .join("projects")
        .join(ClaudeSessionFormat::project_dir(&resolved))
        .join(format!("{session_id}.jsonl"))
}

fn unavailable_session(session_id: &str, diagnostic: String) -> HarnessError {
    HarnessError::new(
        FailureStage::Startup,
        ErrorClass::UnavailableSession,
        SessionImpact::Lost,
        format!("Claude Session {session_id} cannot be resumed: {diagnostic}"),
    )
}

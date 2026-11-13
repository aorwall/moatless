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
    projects_dir(config)
        .await
        .join(format!("{session_id}.jsonl"))
}

async fn projects_dir(config: &Config) -> PathBuf {
    let workspace = Path::new(&config.workspace_dir);
    let resolved = tokio::fs::canonicalize(workspace)
        .await
        .unwrap_or_else(|_| workspace.to_path_buf());
    Path::new(&config.claude_config_dir)
        .join("projects")
        .join(ClaudeSessionFormat::project_dir(&resolved))
}

/// The raw transcript `claude --continue` picks up, the newest in the
/// workspace's project dir; `None` when there is none to continue.
pub(crate) async fn continued_transcript(config: &Config) -> Option<String> {
    let mut entries = tokio::fs::read_dir(projects_dir(config).await).await.ok()?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }
        let Ok(modified) = entry.metadata().await.and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(at, _)| modified > *at) {
            newest = Some((modified, path));
        }
    }
    tokio::fs::read_to_string(newest?.1).await.ok()
}

fn unavailable_session(session_id: &str, diagnostic: String) -> HarnessError {
    HarnessError::new(
        FailureStage::Startup,
        ErrorClass::UnavailableSession,
        SessionImpact::Lost,
        format!("Claude Session {session_id} cannot be resumed: {diagnostic}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_continued_transcript_is_the_newest_session() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = tmp.path().display().to_string();
        config.claude_config_dir = tmp.path().join(".claude").display().to_string();
        assert_eq!(continued_transcript(&config).await, None);

        let dir = projects_dir(&config).await;
        std::fs::create_dir_all(&dir).expect("projects dir");
        let older = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        for (name, text, at) in [
            ("old.jsonl", "old", Some(older)),
            ("new.jsonl", "new", None),
            ("notes.txt", "not a session", None),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, text).expect("write");
            if let Some(at) = at {
                std::fs::File::options()
                    .write(true)
                    .open(&path)
                    .and_then(|file| file.set_modified(at))
                    .expect("set mtime");
            }
        }

        assert_eq!(continued_transcript(&config).await.as_deref(), Some("new"));
    }
}

//! Claude transcript paths, inventory and resume target selection.

use crate::api::options::ClaudeOptions;
use crate::error::{Result, SdkError};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Lexically normalise a raw cwd to match `canonicalize`'s no-trailing-slash
/// form: trim a trailing `/` (preserving root `/`).
///
/// [`resolve_project_dir`]'s dashed encoding is just `replace('/', "-")`, so
/// matching the trailing-slash shape is all that's needed for the names to line
/// up.
fn lexical_cwd(c: &str) -> String {
    let trimmed = c.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Thin wrapper around [`resolve_project_dir`], exposed for the sandbox's
/// fork path.
pub fn transcript_dir(cwd: Option<&str>) -> Result<PathBuf> {
    let options = ClaudeOptions {
        cwd: cwd.map(ToString::to_string),
        ..Default::default()
    };
    resolve_project_dir(&options)
}

/// Resolves `<claude-config-dir>/projects/<dashed-cwd>/`. Config dir
/// precedence: `CLAUDE_CONFIG_DIR` → `CLAUDE_HOME` → `$HOME/.claude`; cwd is
/// dashed by replacing `/` with `-`. `canonicalize` failures fall back to a
/// lexically-normalised cwd instead of erroring, since a transient failure on a
/// good path must not be fatal.
pub(crate) fn resolve_project_dir(options: &ClaudeOptions) -> Result<PathBuf> {
    let cwd_str = if let Some(c) = &options.cwd {
        match std::path::PathBuf::from(c).canonicalize() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => {
                tracing::warn!(
                    cwd = %c,
                    error = %e,
                    "canonicalize(cwd) failed; falling back to lexical path for project-dir resolution"
                );
                lexical_cwd(c)
            }
        }
    } else {
        std::env::current_dir()
            .map_err(|e| SdkError::Internal(format!("std::env::current_dir failed: {e}")))?
            .to_string_lossy()
            .into_owned()
    };

    let dashed = cwd_str.replace('/', "-");

    // Claude writes transcripts under its config dir. We resolve it via the
    // shared helper that mirrors the CLI (`CLAUDE_CONFIG_DIR`, then its
    // `CLAUDE_HOME` alias, then `$HOME/.claude`). The sandbox sets
    // `CLAUDE_CONFIG_DIR` (e.g. `/opt/moatless/.claude`) while `$HOME` is a
    // different path, so honouring it here keeps JSONL discovery pointed at the
    // directory the CLI actually writes to.
    let home = std::env::var("HOME").unwrap_or_default();
    let config_dir = crate::config_dir::resolve_claude_config_dir(&home);

    Ok(config_dir.join("projects").join(dashed))
}

/// Handles resume shapes that append to an EXISTING transcript: `--resume <id>`
/// (`<id>.jsonl`) and `--continue` (most-recently-modified transcript, else
/// long first turns time out waiting for Stop). Returns None for
/// `--fork-session` (new transcript) or no match, leaving discovery to its
/// fallbacks.
pub(crate) async fn resolve_resume_target(
    options: &ClaudeOptions,
    project_dir: &Path,
) -> Option<PathBuf> {
    if options.fork_session == Some(true) {
        return None;
    }
    if let Some(id) = options.resume.as_ref() {
        return Some(project_dir.join(format!("{id}.jsonl")));
    }
    if options.continue_conversation == Some(true) {
        return newest_transcript(project_dir).await;
    }
    None
}

/// The most-recently-modified `*.jsonl` in `dir`, or None if there are none.
pub(crate) async fn newest_transcript(dir: &Path) -> Option<PathBuf> {
    let mut read = tokio::fs::read_dir(dir).await.ok()?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    while let Ok(Some(entry)) = read.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(mtime) = entry.metadata().await.and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| mtime > *t) {
            newest = Some((mtime, path));
        }
    }
    newest.map(|(_, p)| p)
}

pub(crate) async fn list_jsonl_stems(dir: &Path) -> Result<HashSet<String>> {
    let mut set = HashSet::new();
    let mut read = match tokio::fs::read_dir(dir).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(set),
        Err(e) => {
            return Err(SdkError::Internal(format!(
                "Failed to read project dir {}: {e}",
                dir.display()
            )));
        }
    };
    while let Some(entry) = read
        .next_entry()
        .await
        .map_err(|e| SdkError::Internal(format!("read_dir.next_entry failed: {e}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                set.insert(stem.to_string());
            }
        }
    }
    Ok(set)
}

/// True if `dir` contains at least one `*.jsonl` transcript. `NotFound` → false
/// (mirrors [`list_jsonl_stems`]).
pub(crate) async fn has_transcript_in(dir: &Path) -> bool {
    let Ok(mut read) = tokio::fs::read_dir(dir).await else {
        return false;
    };
    while let Ok(Some(entry)) = read.next_entry().await {
        if entry.path().extension().and_then(|s| s.to_str()) == Some("jsonl") {
            return true;
        }
    }
    false
}

/// True when `cwd`'s Claude project dir already has a transcript to resume.
///
/// A cwd canonicalize failure falls back to the lexical path rather than
/// erroring, so this returns `Ok(false)` (start fresh) instead of `Err`;
/// `Result` is kept for the remaining I/O-error surface.
pub async fn has_resumable_transcript(cwd: Option<&str>) -> Result<bool> {
    let options = ClaudeOptions {
        cwd: cwd.map(ToString::to_string),
        ..Default::default()
    };
    let project_dir = resolve_project_dir(&options)?;
    Ok(has_transcript_in(&project_dir).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The env-var precedence itself is covered by `crate::config_dir` tests;
    // here we only assert that `resolve_project_dir` joins `projects/<dashed-cwd>`
    // under the resolved config dir. Mutating process-global env vars must run
    // serially, so this is a single test.
    #[test]
    fn project_dir_joins_projects_under_config_dir() {
        let _guard = crate::config_dir::ENV_LOCK.lock().expect("env lock");
        // Use an existing directory so canonicalize() succeeds — tempdir works.
        let tmp = std::env::temp_dir();
        let canonical = tmp.canonicalize().expect("temp_dir should canonicalize");
        let cwd_str = canonical.to_string_lossy().into_owned();
        let dashed = cwd_str.replace('/', "-");
        let options = ClaudeOptions {
            cwd: Some(cwd_str.clone()),
            ..Default::default()
        };

        let config_dir = tmp.join("tmux-test-config");
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);
        let resolved = resolve_project_dir(&options).expect("resolve");
        assert_eq!(resolved, config_dir.join("projects").join(&dashed));

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    // When `canonicalize(cwd)` fails (here `ENOENT` on a nonexistent path,
    // exercising the same fallback branch as the `EACCES` SELinux-relabel race),
    // resolution must NOT error — it falls back to the lexically-normalised raw
    // cwd so transcript discovery still targets `projects/<dashed-cwd>`.
    #[test]
    fn project_dir_falls_back_when_cwd_not_canonicalizable() {
        let _guard = crate::config_dir::ENV_LOCK.lock().expect("env lock");
        let options = ClaudeOptions {
            cwd: Some("/nonexistent/abc".into()),
            ..Default::default()
        };

        let config_dir = std::env::temp_dir().join("tmux-test-config-fallback");
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);
        let resolved = resolve_project_dir(&options).expect("resolve falls back, not errors");
        assert_eq!(
            resolved,
            config_dir.join("projects").join("-nonexistent-abc")
        );

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[tokio::test]
    async fn has_transcript_in_empty_dir_is_false() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        assert!(!has_transcript_in(tmp.path()).await);
    }

    #[tokio::test]
    async fn has_transcript_in_dir_with_jsonl_is_true() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        // Non-jsonl files must not count as transcripts.
        std::fs::write(tmp.path().join("notes.txt"), "x").expect("write notes.txt");
        assert!(!has_transcript_in(tmp.path()).await);
        std::fs::write(tmp.path().join("x.jsonl"), "{}").expect("write x.jsonl");
        assert!(has_transcript_in(tmp.path()).await);
    }

    #[tokio::test]
    async fn has_transcript_in_missing_dir_is_false() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        assert!(!has_transcript_in(&tmp.path().join("does-not-exist")).await);
    }

    // Asserts the probe resolves the same `projects/<dashed-cwd>` dir under
    // `CLAUDE_CONFIG_DIR` that the tmux transport polls. Mutates process-global
    // env, so it runs under the shared env lock — a sync test with `block_on`
    // rather than `#[tokio::test]`, to avoid holding the guard across awaits.
    #[test]
    fn has_resumable_transcript_resolves_env_config_dir() {
        let _guard = crate::config_dir::ENV_LOCK.lock().expect("env lock");
        // Use an existing directory so canonicalize() succeeds — tempdir works.
        let cwd = std::env::temp_dir();
        let canonical = cwd.canonicalize().expect("temp_dir should canonicalize");
        let dashed = canonical.to_string_lossy().replace('/', "-");

        let config_dir = tempfile::tempdir().expect("create tempdir");
        std::env::set_var("CLAUDE_CONFIG_DIR", config_dir.path());

        let project_dir = config_dir.path().join("projects").join(&dashed);
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let transcript = project_dir.join("s.jsonl");
        std::fs::write(&transcript, "{}").expect("write transcript");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");
        let cwd_str = cwd.to_string_lossy().into_owned();
        let probe = |cwd: &str| rt.block_on(has_resumable_transcript(Some(cwd)));
        assert!(probe(&cwd_str).expect("probe"));

        std::fs::remove_file(&transcript).expect("remove transcript");
        assert!(!probe(&cwd_str).expect("probe"));

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[tokio::test]
    async fn resume_target_resume_id_points_at_id_jsonl() {
        let dir = std::env::temp_dir();
        let options = ClaudeOptions {
            resume: Some("abc-123".to_string()),
            ..Default::default()
        };
        let target = resolve_resume_target(&options, &dir).await;
        assert_eq!(target, Some(dir.join("abc-123.jsonl")));
    }

    #[tokio::test]
    async fn resume_target_fork_is_none() {
        let dir = std::env::temp_dir();
        let options = ClaudeOptions {
            resume: Some("abc-123".to_string()),
            fork_session: Some(true),
            ..Default::default()
        };
        assert_eq!(resolve_resume_target(&options, &dir).await, None);
    }

    #[tokio::test]
    async fn resume_target_plain_query_is_none() {
        let dir = std::env::temp_dir();
        assert_eq!(
            resolve_resume_target(&ClaudeOptions::default(), &dir).await,
            None
        );
    }

    /// Create a unique empty temp subdir (no `tempfile` dev-dep in this crate).
    fn unique_tmp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sdk-resume-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[tokio::test]
    // `from_secs(60)` reads more clearly than the lint's `from_mins(1)` for a
    // one-minute mtime offset.
    #[allow(clippy::duration_suboptimal_units)]
    async fn resume_target_continue_picks_newest_existing_transcript() {
        let dir = unique_tmp_dir();
        // Two transcripts with explicit, distinct mtimes; --continue must pick
        // the newest (the one claude reopens).
        let older = dir.join("older.jsonl");
        let newer = dir.join("newer.jsonl");
        std::fs::write(&older, b"{}\n").expect("write older");
        std::fs::write(&newer, b"{}\n").expect("write newer");
        let base = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::open(&older)
            .unwrap()
            .set_modified(base)
            .expect("set older mtime");
        std::fs::File::open(&newer)
            .unwrap()
            .set_modified(base + std::time::Duration::from_secs(60))
            .expect("set newer mtime");
        // A non-jsonl file must be ignored.
        std::fs::write(dir.join("notes.txt"), b"x").expect("write txt");

        let options = ClaudeOptions {
            continue_conversation: Some(true),
            ..Default::default()
        };
        assert_eq!(resolve_resume_target(&options, &dir).await, Some(newer));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn resume_target_continue_empty_dir_is_none() {
        let dir = unique_tmp_dir();
        let options = ClaudeOptions {
            continue_conversation: Some(true),
            ..Default::default()
        };
        assert_eq!(resolve_resume_target(&options, &dir).await, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}

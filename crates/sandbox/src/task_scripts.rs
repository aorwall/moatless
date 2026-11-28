//! Scripts declared on one Member rather than on its Workspace.
//!
//! A Workspace Script is a column on the `workspaces` row, so declaring one
//! puts a control on every Task in that Workspace. These are the ad-hoc half: a
//! file under `WORKSPACE_DIR`, so it rides the snapshot into a replacement pod
//! and no database copy exists. `moatless_t3code::scripts` merges them over the
//! Workspace's own and runs either through one path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokio::sync::Mutex;
use tracing::warn;

/// Serializes the read-modify-write both writers perform.
///
/// tonic serves concurrently, and one file holds every Script for the Task, so
/// an unguarded pair of declares would drop whichever was read first.
static SCRIPTS_LOCK: Mutex<()> = Mutex::const_new(());

/// One declared Script, in the fields `ScriptConfig` carries.
///
/// No `env_vars`: the surface that declares these is read by a task-scoped
/// bearer key, which does not carry the workspace-level access a pinned secret
/// would assume.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskScript {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default)]
    pub run_on_start: bool,
}

/// A Task's Scripts, keyed by [`TaskScript::name`].
pub type TaskScripts = BTreeMap<String, TaskScript>;

/// Reject names that cannot be carried by everything downstream. The name
/// becomes the terminal session id `script-<name>` and the preview tab id, and
/// it reaches us over gRPC, so it is untrusted input.
pub fn is_valid_script_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Where a Task's declared Scripts live.
///
/// A sibling of the checkouts under `WORKSPACE_DIR` so it stays out of the
/// user's `git status`, and keyed by Task so two Members sharing a checkout
/// don't overwrite each other's — the same shape as
/// [`crate::commands::default_journal_path`].
pub fn path(workspace_dir: &str, task_id: &str) -> PathBuf {
    Path::new(workspace_dir).join(format!(".moat-scripts-{task_id}.json"))
}

/// Declare `script`, replacing whatever was held under its name.
///
/// Whole-state: this never merges a delta into the stored Script.
pub async fn apply(
    workspace_dir: &str,
    task_id: &str,
    script: TaskScript,
) -> std::io::Result<TaskScripts> {
    if !is_valid_script_name(&script.name) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid script name: {}", script.name),
        ));
    }
    let path = path(workspace_dir, task_id);
    let _guard = SCRIPTS_LOCK.lock().await;
    let mut scripts = read(&path).await;
    scripts.insert(script.name.clone(), script);
    write(&path, &scripts).await?;
    Ok(scripts)
}

/// Drop `name`. Removing one that was never declared succeeds, so a caller
/// retrying a remove it already completed is not told it failed.
pub async fn remove(
    workspace_dir: &str,
    task_id: &str,
    name: &str,
) -> std::io::Result<TaskScripts> {
    let path = path(workspace_dir, task_id);
    let _guard = SCRIPTS_LOCK.lock().await;
    let mut scripts = read(&path).await;
    scripts.remove(name);
    // Written even when the name was absent: a file [`read`] could not parse
    // reads as no Scripts, and this is what clears it rather than leaving it for
    // the next declare.
    write(&path, &scripts).await?;
    Ok(scripts)
}

/// Every Script this Task has declared.
///
/// An absent file means none were, and an unreadable one is reported as none:
/// this feeds a merge whose other half is the Workspace's Scripts, and failing
/// the read would take those down with it.
pub async fn read_all(workspace_dir: &str, task_id: &str) -> TaskScripts {
    let path = path(workspace_dir, task_id);
    let _guard = SCRIPTS_LOCK.lock().await;
    read(&path).await
}

/// Read with the lock already held.
async fn read(path: &Path) -> TaskScripts {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %path.display(), %error, "Failed to read the task's scripts");
            }
            return TaskScripts::new();
        }
    };
    match serde_json::from_slice::<TaskScripts>(&bytes) {
        Ok(scripts) => scripts,
        Err(error) => {
            warn!(path = %path.display(), %error, "Discarding unreadable task scripts");
            TaskScripts::new()
        }
    }
}

async fn write(path: &Path, scripts: &TaskScripts) -> std::io::Result<()> {
    if scripts.is_empty() {
        if let Err(error) = tokio::fs::remove_file(path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error);
        }
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(scripts)?;
    // Sibling temp file and rename, as `agent::wake_note::write` does: `rename`
    // is atomic within a filesystem, so a kill or a snapshot taken mid-write
    // sees either file whole rather than the truncated one `write` would leave
    // — which `read` would then discard, losing every Script at once.
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &bytes).await?;
    if let Err(error) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway workspace dir, so two tests never share a file.
    fn scratch() -> String {
        let dir = std::env::temp_dir().join(format!("moat-task-scripts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir.display().to_string()
    }

    fn script(name: &str, command: &str) -> TaskScript {
        TaskScript {
            name: name.to_string(),
            command: command.to_string(),
            label: None,
            icon: None,
            port: None,
            run_on_start: false,
        }
    }

    #[tokio::test]
    async fn a_declared_script_reads_back_and_removes() {
        let dir = scratch();
        apply(
            &dir,
            "task-a",
            script("demo", "python3 -m http.server 4321"),
        )
        .await
        .expect("apply");

        let stored = read_all(&dir, "task-a").await;
        assert_eq!(stored["demo"].command, "python3 -m http.server 4321");

        remove(&dir, "task-a", "demo").await.expect("remove");
        assert!(read_all(&dir, "task-a").await.is_empty());
    }

    #[tokio::test]
    async fn a_task_that_declared_nothing_reads_as_empty() {
        assert!(read_all(&scratch(), "task-a").await.is_empty());
    }

    /// Removing what was never declared is how a retried remove behaves, and it
    /// must not be reported as a failure.
    #[tokio::test]
    async fn removing_an_undeclared_script_succeeds() {
        remove(&scratch(), "task-a", "demo").await.expect("remove");
    }

    /// A half-written or hand-edited file reads as no Scripts, so nothing else
    /// would ever clear it: a remove is the one call that must.
    #[tokio::test]
    async fn removing_clears_a_file_that_cannot_be_parsed() {
        let dir = scratch();
        let path = path(&dir, "task-a");
        tokio::fs::write(&path, b"{ not json").await.expect("write");

        remove(&dir, "task-a", "demo").await.expect("remove");

        assert!(!path.exists(), "the unparseable file was left in place");
    }

    #[tokio::test]
    async fn a_second_declare_replaces_the_first() {
        let dir = scratch();
        apply(&dir, "task-a", script("demo", "first"))
            .await
            .expect("first");
        let stored = apply(&dir, "task-a", script("demo", "second"))
            .await
            .expect("second");

        assert_eq!(stored.len(), 1);
        assert_eq!(stored["demo"].command, "second");
    }

    /// Two Members can share one `WORKSPACE_DIR`, so a Script declared by one
    /// must not appear on the other.
    #[tokio::test]
    async fn each_member_keeps_its_own_scripts() {
        let dir = scratch();
        apply(&dir, "task-a", script("demo", "a")).await.expect("a");
        apply(&dir, "task-b", script("demo", "b")).await.expect("b");

        assert_eq!(read_all(&dir, "task-a").await["demo"].command, "a");
        assert_eq!(read_all(&dir, "task-b").await["demo"].command, "b");
    }

    #[tokio::test]
    async fn concurrent_declares_both_survive() {
        let dir = scratch();
        let (a, b) = tokio::join!(
            apply(&dir, "task-a", script("one", "a")),
            apply(&dir, "task-a", script("two", "b")),
        );
        a.expect("a");
        b.expect("b");

        let stored = read_all(&dir, "task-a").await;
        assert_eq!(stored.len(), 2, "one declare was overwritten: {stored:?}");
    }

    #[tokio::test]
    async fn a_name_that_would_not_survive_the_terminal_id_is_refused() {
        for name in ["", "../escape", "has space", "semi;colon"] {
            apply(&scratch(), "task-a", script(name, "echo hi"))
                .await
                .expect_err(name);
        }
    }
}

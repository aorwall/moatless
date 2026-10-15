//! Wake notes: text an in-pod caller leaves for the agent, held across a pod
//! replacement and delivered once when the pod is next open for work.
//!
//! A caller that takes the pod down cannot report what it did afterwards — the
//! process issuing the request dies with the pod. `moat tasks stop`, `restart`
//! and `redeploy` with `--yes` are such callers. Each records the note first, and
//! [`crate::agent::restart::open_for_work`] delivers it on the next start.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokio::sync::Mutex;
use tracing::warn;

/// How long a note stays deliverable.
///
/// Past this, the caller either never caused the replacement or the agent has
/// long since moved on; also bounds the note a transport error deliberately
/// keeps (`cancel_wake_note` in `moatless-cli`).
const MAX_AGE_MS: i64 = 60 * 60 * 1000;

/// One caller's note, waiting for the agent it names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeNote {
    /// What the agent is told. The caller owns the wording.
    pub text: String,
    /// When [`record`] stored it, against which [`take`] applies
    /// [`MAX_AGE_MS`].
    pub recorded_at_unix_ms: i64,
}

/// Every Member's note, keyed by `task_id`.
///
/// A Sandbox has several Members, each driving its own agent; keying by Member
/// is what makes [`cancel`] unambiguous when two record at once.
type Notes = BTreeMap<String, WakeNote>;

/// Serializes the read-modify-write every entry point performs.
///
/// tonic serves concurrently, so an unguarded read-modify-write could drop
/// whichever note was read first; one lock covers the pod's single notes file.
static NOTES_LOCK: Mutex<()> = Mutex::const_new(());

/// Where the pod's wake notes live. Inside `WORKSPACE_DIR` so they ride the
/// snapshot into a replacement pod, and a sibling of the checkouts rather than
/// a path inside one, so they stay out of the user's `git status`.
pub fn default_path(workspace_dir: &str) -> PathBuf {
    Path::new(workspace_dir).join(".moat-wake-notes.json")
}

/// Hold `text` for `task_id`, replacing any note that Member already left; a
/// Member has at most one, since it can cause only one replacement.
///
/// An error means nothing was stored — the caller must not take the pod down
/// after one, or the agent is never told why.
pub async fn record(path: &Path, task_id: &str, text: String) -> std::io::Result<()> {
    let _guard = NOTES_LOCK.lock().await;
    let mut notes = read(path).await;
    notes.insert(
        task_id.to_string(),
        WakeNote {
            text,
            recorded_at_unix_ms: now_unix_ms(),
        },
    );
    write(path, &notes).await
}

/// Drop `task_id`'s note.
///
/// The caller records before it acts, since a pod going down cannot be told
/// anything afterward; an action that then fails outright must take its note
/// back, or the next unrelated replacement delivers it.
pub async fn cancel(path: &Path, task_id: &str) -> std::io::Result<()> {
    let _guard = NOTES_LOCK.lock().await;
    let mut notes = read(path).await;
    if notes.remove(task_id).is_none() {
        return Ok(());
    }
    write(path, &notes).await
}

/// Read and clear every note a previous incarnation of this pod left behind,
/// dropping any older than [`MAX_AGE_MS`].
///
/// Clearing here delivers a note exactly once: the file is gone whether or not
/// it parsed, so a corrupt file isn't re-read on every later start.
pub async fn take(path: &Path) -> Notes {
    let _guard = NOTES_LOCK.lock().await;
    let mut notes = read(path).await;
    if let Err(error) = remove(path).await {
        warn!(path = %path.display(), %error, "Failed to clear the wake notes");
    }
    let now = now_unix_ms();
    notes.retain(|task_id, note| {
        let fresh = now.saturating_sub(note.recorded_at_unix_ms) <= MAX_AGE_MS;
        if !fresh {
            warn!(%task_id, "Dropping a wake note that is too old to announce");
        }
        fresh
    });
    notes
}

async fn read(path: &Path) -> Notes {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %path.display(), %error, "Failed to read the wake notes");
            }
            return Notes::new();
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(notes) => notes,
        Err(error) => {
            warn!(path = %path.display(), %error, "Discarding unreadable wake notes");
            Notes::new()
        }
    }
}

async fn write(path: &Path, notes: &Notes) -> std::io::Result<()> {
    if notes.is_empty() {
        return remove(path).await;
    }
    let bytes = serde_json::to_vec(notes)?;
    // Write to a sibling temp file and rename over the target. `rename` is
    // atomic within a filesystem, so a kill or a snapshot taken mid-write sees
    // either the old file or the new one whole, never the truncated file
    // `write` would leave — which `take` would then discard, losing the very
    // wake this exists to deliver.
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &bytes).await?;
    if let Err(error) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    Ok(())
}

/// Delete the notes file; an absent file is already the wanted state.
async fn remove(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway path in its own directory, so two tests never share a file.
    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moat-wake-notes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        default_path(&dir.display().to_string())
    }

    #[tokio::test]
    async fn a_recorded_note_is_delivered_once() {
        let path = scratch();
        record(&path, "task-a", "the restart finished".into())
            .await
            .expect("record");

        let taken = take(&path).await;
        assert_eq!(taken["task-a"].text, "the restart finished");
        assert!(take(&path).await.is_empty());
    }

    #[tokio::test]
    async fn a_second_note_replaces_the_first() {
        let path = scratch();
        record(&path, "task-a", "first".into())
            .await
            .expect("first");
        record(&path, "task-a", "second".into())
            .await
            .expect("second");

        let taken = take(&path).await;
        assert_eq!(taken.len(), 1);
        assert_eq!(taken["task-a"].text, "second");
    }

    /// Members are peers, so one Member's note must not stand in for another's.
    #[tokio::test]
    async fn each_member_keeps_its_own_note() {
        let path = scratch();
        record(&path, "task-a", "a".into()).await.expect("a");
        record(&path, "task-b", "b".into()).await.expect("b");

        let taken = take(&path).await;
        assert_eq!(taken["task-a"].text, "a");
        assert_eq!(taken["task-b"].text, "b");
    }

    /// Two Members record at once, and neither may lose to the other's write.
    #[tokio::test]
    async fn concurrent_records_both_survive() {
        let path = scratch();
        let (a, b) = tokio::join!(
            record(&path, "task-a", "a".into()),
            record(&path, "task-b", "b".into()),
        );
        a.expect("a");
        b.expect("b");

        let taken = take(&path).await;
        assert_eq!(taken.len(), 2, "one note was overwritten: {taken:?}");
    }

    #[tokio::test]
    async fn cancelling_one_member_leaves_the_other() {
        let path = scratch();
        record(&path, "task-a", "a".into()).await.expect("a");
        record(&path, "task-b", "b".into()).await.expect("b");
        cancel(&path, "task-a").await.expect("cancel");

        let taken = take(&path).await;
        assert_eq!(taken.len(), 1);
        assert!(taken.contains_key("task-b"));
    }

    #[tokio::test]
    async fn cancelling_the_last_note_leaves_no_file() {
        let path = scratch();
        record(&path, "task-a", "a".into()).await.expect("a");
        cancel(&path, "task-a").await.expect("cancel");

        assert!(!path.exists());
        assert!(take(&path).await.is_empty());
    }

    /// `CancelWakeNote` answers with this result, so a note still on disk
    /// must not come back as `Ok`.
    #[tokio::test]
    async fn emptying_the_notes_reports_a_delete_that_failed() {
        let path = scratch();
        // A directory where the file should be: `remove_file` refuses it.
        tokio::fs::create_dir(&path).await.expect("mkdir");

        assert!(write(&path, &Notes::new()).await.is_err());
        assert!(write(&scratch(), &Notes::new()).await.is_ok());
    }

    #[tokio::test]
    async fn an_absent_file_yields_no_notes() {
        assert!(take(&scratch()).await.is_empty());
    }

    /// A corrupt file must not be re-read on every later start.
    #[tokio::test]
    async fn an_unreadable_file_is_discarded() {
        let path = scratch();
        tokio::fs::write(&path, b"{not json").await.expect("write");

        assert!(take(&path).await.is_empty());
        assert!(!path.exists());
    }

    /// A note nobody collected must not announce a restart hours after it.
    #[tokio::test]
    async fn a_stale_note_is_dropped() {
        let path = scratch();
        let notes = Notes::from([(
            "task-a".to_string(),
            WakeNote {
                text: "a".to_string(),
                recorded_at_unix_ms: now_unix_ms() - MAX_AGE_MS - 1,
            },
        )]);
        tokio::fs::write(&path, serde_json::to_vec(&notes).expect("encode"))
            .await
            .expect("write");

        assert!(take(&path).await.is_empty());
    }

    /// An undeliverable path is an error, so a caller never takes the pod down
    /// believing the agent will be told why.
    #[tokio::test]
    async fn a_note_that_cannot_be_stored_reports_the_failure() {
        let path = scratch().join("not-a-directory").join("notes.json");

        assert!(record(&path, "task-a", "a".into()).await.is_err());
    }
}

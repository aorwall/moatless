//! Which copy of a Task's files a read is answered from: its Sandbox while one
//! is running, the stored workspace snapshot once none is.
//!
//! Only the choice lives here. What each copy is asked, the response it
//! becomes and how an error reads back stay with the read surface.

use moatless_ports::sandbox::WorkspaceSnapshotSource;
use moatless_types::sandbox::{SandboxError, no_live_sandbox};
use sandbox_proto::client::{SandboxClient, SandboxConnector};

/// The copy a read goes to.
pub enum WorkspaceCopy<'a> {
    Live(SandboxClient),
    /// `unreachable` is what the Sandbox said: the answer when the snapshot
    /// holds nothing either, since no files still means nothing is running.
    Stored {
        source: &'a dyn WorkspaceSnapshotSource,
        unreachable: SandboxError,
    },
}

/// Why no copy could be opened.
#[derive(Debug)]
pub enum OpenError {
    /// No Sandbox is running and the deployment stores no snapshots.
    NotRunning(SandboxError),
    /// The Sandbox failed in a way a stored copy must not paper over.
    Sandbox(SandboxError),
}

impl OpenError {
    /// The connector's error, for a caller that answers both cases alike.
    pub fn into_sandbox_error(self) -> SandboxError {
        match self {
            OpenError::NotRunning(error) | OpenError::Sandbox(error) => error,
        }
    }
}

/// Open `task_id`'s files, live when its Sandbox answers and from `snapshots`
/// when none is running.
///
/// [`no_live_sandbox`] is the whole distinction: a Sandbox that is up and
/// refused the connection has current files, so a stored copy would present
/// an older tree as live.
pub async fn open<'a>(
    sandboxes: &dyn SandboxConnector,
    snapshots: Option<&'a dyn WorkspaceSnapshotSource>,
    task_id: &str,
) -> Result<WorkspaceCopy<'a>, OpenError> {
    match sandboxes.sandbox_client(task_id).await {
        Ok(client) => Ok(WorkspaceCopy::Live(client)),
        Err(error) if no_live_sandbox(&error) => match snapshots {
            Some(source) => Ok(WorkspaceCopy::Stored {
                source,
                unreachable: error,
            }),
            None => Err(OpenError::NotRunning(error)),
        },
        Err(error) => Err(OpenError::Sandbox(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use moatless_ports::sandbox::MockWorkspaceSnapshotSource;

    /// A connector that answers with a lazy client, or with the error it builds.
    struct Answers(Option<fn() -> SandboxError>);

    #[async_trait]
    impl SandboxConnector for Answers {
        async fn sandbox_client(&self, _task_id: &str) -> Result<SandboxClient, SandboxError> {
            match self.0 {
                None => Ok(SandboxClient::connect("http://127.0.0.1:1").expect("lazy client")),
                Some(fail) => Err(fail()),
            }
        }
    }

    #[tokio::test]
    async fn a_running_sandbox_is_read_live_even_with_a_snapshot() {
        let source = MockWorkspaceSnapshotSource::new();
        let copy = open(&Answers(None), Some(&source), "task-1").await;
        assert!(matches!(copy, Ok(WorkspaceCopy::Live(_))));
    }

    #[tokio::test]
    async fn no_live_sandbox_falls_back_to_the_snapshot() {
        let source = MockWorkspaceSnapshotSource::new();
        let stopped = Answers(Some(|| SandboxError::NotReady("stopped".into())));
        let copy = open(&stopped, Some(&source), "task-1").await;
        assert!(matches!(
            copy,
            Ok(WorkspaceCopy::Stored {
                unreachable: SandboxError::NotReady(_),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn no_live_sandbox_and_no_snapshots_is_not_running() {
        let stopped = Answers(Some(|| SandboxError::NotFound("gone".into())));
        let copy = open(&stopped, None, "task-1").await;
        assert!(matches!(
            copy,
            Err(OpenError::NotRunning(SandboxError::NotFound(_)))
        ));
    }

    /// A Sandbox that is up but failed has newer files than any snapshot.
    #[tokio::test]
    async fn any_other_sandbox_failure_skips_the_snapshot() {
        let source = MockWorkspaceSnapshotSource::new();
        let failing = Answers(Some(|| SandboxError::BadRequest("refused".into())));
        let copy = open(&failing, Some(&source), "task-1").await;
        assert!(matches!(
            copy,
            Err(OpenError::Sandbox(SandboxError::BadRequest(_)))
        ));
    }
}

/// A child's nonzero exit status, which `main` exits with instead of printing
/// an error.
#[derive(Debug, thiserror::Error)]
#[error("exited with status {0}")]
pub struct CommandExit(pub i32);

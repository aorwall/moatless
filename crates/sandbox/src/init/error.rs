/// Init error with permanence classification.
/// Permanent errors use exit code 78 so the pod watcher stops immediately.
/// Transient errors use exit code 1 and let K8s retry.
#[derive(Debug)]
pub enum InitError {
    /// Will never succeed on retry (repo not found, access denied).
    Permanent(String),
    /// Might succeed on retry (backend unreachable, network timeout).
    Transient(String),
}

impl std::error::Error for InitError {}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Permanent(msg) | Self::Transient(msg) => f.write_str(msg),
        }
    }
}

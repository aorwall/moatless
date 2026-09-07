/// Errors from file operations.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("File not found: {0}")]
    NotFound(String),
    #[error("Path is a directory: {0}")]
    IsDirectory(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

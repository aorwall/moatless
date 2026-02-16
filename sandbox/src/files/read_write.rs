//! File read and write operations within the sandbox workspace.

use tokio::fs;

use super::git;
use super::{validate_path, FileError};

/// Result of reading a file with optional git change info.
#[derive(Debug, Default)]
pub struct ReadFileResult {
    pub content: String,
    pub old_content: Option<String>,
    pub additions: Option<i32>,
    pub deletions: Option<i32>,
    pub is_modified: Option<bool>,
}

/// Read a file from the workspace.
pub async fn read_file(workspace_dir: &str, path: &str) -> Result<String, FileError> {
    let full_path = validate_path(workspace_dir, path)?;

    if !full_path.exists() {
        return Err(FileError::NotFound(path.to_string()));
    }

    let content = fs::read_to_string(&full_path).await?;
    Ok(content)
}

/// Write content to a file in the workspace.
///
/// Creates parent directories if they don't exist.
pub async fn write_file(workspace_dir: &str, path: &str, content: &str) -> Result<(), FileError> {
    let full_path = validate_path(workspace_dir, path)?;

    // Create parent directories if needed
    if let Some(parent) = full_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).await?;
        }
    }

    fs::write(&full_path, content).await?;
    Ok(())
}

/// Read a file and include its git change information.
///
/// If git is unavailable or the file has no changes, the change fields
/// are `None` / zero.
pub async fn read_file_with_changes(
    workspace_dir: &str,
    path: &str,
) -> Result<ReadFileResult, FileError> {
    let content = read_file(workspace_dir, path).await?;

    let change_info = git::get_file_change_info(workspace_dir, path).await;

    if change_info.is_modified {
        Ok(ReadFileResult {
            content,
            old_content: change_info.old_content,
            additions: Some(change_info.additions),
            deletions: Some(change_info.deletions),
            is_modified: Some(true),
        })
    } else {
        Ok(ReadFileResult {
            content,
            old_content: None,
            additions: None,
            deletions: None,
            is_modified: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn read_existing_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("hello.txt"), "world").unwrap();

        let content = read_file(tmp.path().to_str().unwrap(), "hello.txt")
            .await
            .unwrap();
        assert_eq!(content, "world");
    }

    #[tokio::test]
    async fn read_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let result = read_file(tmp.path().to_str().unwrap(), "nope.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn write_new_file() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path().to_str().unwrap(), "out.txt", "hello")
            .await
            .unwrap();

        let content = std::fs::read_to_string(tmp.path().join("out.txt")).unwrap();
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn write_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path().to_str().unwrap(), "a/b/c.txt", "nested")
            .await
            .unwrap();

        let content = std::fs::read_to_string(tmp.path().join("a/b/c.txt")).unwrap();
        assert_eq!(content, "nested");
    }

    #[tokio::test]
    async fn write_rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let result = write_file(tmp.path().to_str().unwrap(), "../escape.txt", "bad").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_file_with_changes_no_git() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("file.txt"), "content").unwrap();

        let result = read_file_with_changes(tmp.path().to_str().unwrap(), "file.txt")
            .await
            .unwrap();
        assert_eq!(result.content, "content");
        // No git repo, so no changes
        assert!(result.old_content.is_none());
        assert!(result.is_modified.is_none());
    }
}

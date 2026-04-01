//! File read and write operations within the sandbox workspace.

use base64::Engine;
use tokio::fs;

use super::git;
use super::{validate_path, FileError};

/// Result of reading a file (may be text or base64-encoded binary).
#[derive(Debug)]
pub struct ReadResult {
    pub content: String,
    pub is_binary: bool,
    pub media_type: Option<String>,
}

/// Result of reading a file with optional git change info.
#[derive(Debug, Default)]
pub struct ReadFileResult {
    pub content: String,
    pub old_content: Option<String>,
    pub additions: Option<i32>,
    pub deletions: Option<i32>,
    pub is_modified: Option<bool>,
    pub is_binary: bool,
    pub media_type: Option<String>,
}

/// Detect media type from file extension.
fn media_type_from_extension(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?.to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => return None,
    };
    Some(mime.to_string())
}

/// Read a file from the workspace, falling back to base64 for binary files.
pub async fn read_file(workspace_dir: &str, path: &str) -> Result<ReadResult, FileError> {
    let full_path = validate_path(workspace_dir, path)?;

    if !full_path.exists() {
        return Err(FileError::NotFound(path.to_string()));
    }

    match fs::read_to_string(&full_path).await {
        Ok(content) => Ok(ReadResult {
            content,
            is_binary: false,
            media_type: None,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            // Not valid UTF-8 — treat as binary
            let bytes = fs::read(&full_path).await?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok(ReadResult {
                content: encoded,
                is_binary: true,
                media_type: media_type_from_extension(path),
            })
        }
        Err(e) => Err(FileError::Io(e)),
    }
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
/// are `None` / zero. Binary files skip git change detection.
pub async fn read_file_with_changes(
    workspace_dir: &str,
    path: &str,
) -> Result<ReadFileResult, FileError> {
    let result = read_file(workspace_dir, path).await?;

    // Skip git diffs for binary files — they're not meaningful
    if result.is_binary {
        return Ok(ReadFileResult {
            content: result.content,
            is_binary: true,
            media_type: result.media_type,
            ..Default::default()
        });
    }

    let change_info = git::get_file_change_info(workspace_dir, path).await;

    if change_info.is_modified {
        Ok(ReadFileResult {
            content: result.content,
            old_content: change_info.old_content,
            additions: Some(change_info.additions),
            deletions: Some(change_info.deletions),
            is_modified: Some(true),
            is_binary: false,
            media_type: None,
        })
    } else {
        Ok(ReadFileResult {
            content: result.content,
            old_content: None,
            additions: None,
            deletions: None,
            is_modified: None,
            is_binary: false,
            media_type: None,
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

        let result = read_file(tmp.path().to_str().unwrap(), "hello.txt")
            .await
            .unwrap();
        assert_eq!(result.content, "world");
        assert!(!result.is_binary);
        assert!(result.media_type.is_none());
    }

    #[tokio::test]
    async fn read_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let result = read_file(tmp.path().to_str().unwrap(), "nope.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_binary_file_returns_base64() {
        let tmp = TempDir::new().unwrap();
        // Write invalid UTF-8 bytes with PNG-like extension
        let bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xFE];
        std::fs::write(tmp.path().join("image.png"), &bytes).unwrap();

        let result = read_file(tmp.path().to_str().unwrap(), "image.png")
            .await
            .unwrap();
        assert!(result.is_binary);
        assert_eq!(result.media_type.as_deref(), Some("image/png"));

        // Verify base64 round-trips
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&result.content)
            .unwrap();
        assert_eq!(decoded, bytes);
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
        assert!(!result.is_binary);
        // No git repo, so no changes
        assert!(result.old_content.is_none());
        assert!(result.is_modified.is_none());
    }
}

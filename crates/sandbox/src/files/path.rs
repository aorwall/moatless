use super::FileError;
use std::path::{Path, PathBuf};

/// Validate and resolve a relative path within the workspace.
///
/// Rejects `..` segments, absolute paths, and paths that escape the
/// workspace root after resolution.
pub fn validate_path(workspace_dir: &str, relative_path: &str) -> Result<PathBuf, FileError> {
    // Reject absolute paths
    if relative_path.starts_with('/') {
        return Err(FileError::InvalidPath(
            "Absolute paths are not allowed".to_string(),
        ));
    }

    // Reject path traversal
    if relative_path.contains("..") {
        return Err(FileError::InvalidPath(
            "Path must not contain '..'".to_string(),
        ));
    }

    let workspace = Path::new(workspace_dir);
    let full_path = workspace.join(relative_path);

    // Canonicalize the workspace root (must exist)
    let canonical_workspace = workspace.canonicalize().map_err(|_| {
        FileError::InvalidPath(format!("Workspace directory not found: {workspace_dir}"))
    })?;

    // For the target, try to canonicalize. If the file doesn't exist yet
    // (write case), canonicalize the parent and append the filename.
    let canonical_target =
        if full_path.exists() {
            full_path.canonicalize()?
        } else {
            let parent = full_path.parent().ok_or_else(|| {
                FileError::InvalidPath("Cannot determine parent directory".to_string())
            })?;
            if parent.exists() {
                let canonical_parent = parent.canonicalize()?;
                canonical_parent.join(full_path.file_name().ok_or_else(|| {
                    FileError::InvalidPath("Cannot determine filename".to_string())
                })?)
            } else {
                // Parent doesn't exist either — reconstruct using canonical
                // workspace + relative path so the starts_with check works.
                canonical_workspace.join(relative_path)
            }
        };

    // Ensure the resolved path is within the workspace
    if !canonical_target.starts_with(&canonical_workspace) {
        return Err(FileError::InvalidPath(
            "Path resolves outside workspace".to_string(),
        ));
    }

    Ok(full_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn validate_path_rejects_absolute() {
        let tmp = TempDir::new().unwrap();
        let result = validate_path(tmp.path().to_str().unwrap(), "/etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn validate_path_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let result = validate_path(tmp.path().to_str().unwrap(), "foo/../../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn validate_path_accepts_valid() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("hello.txt"), "hi").unwrap();
        let result = validate_path(tmp.path().to_str().unwrap(), "hello.txt");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_path_accepts_nonexistent_file_in_existing_dir() {
        let tmp = TempDir::new().unwrap();
        let result = validate_path(tmp.path().to_str().unwrap(), "new_file.txt");
        assert!(result.is_ok());
    }
}

//! File name search within the sandbox workspace.

use super::tree;

/// A file name match result.
#[derive(Debug, Clone)]
pub struct FileNameMatch {
    pub path: String,
    pub matches: i32,
}

/// Search for files by substring match on the file path.
///
/// Lists all files recursively then filters by substring. Skips
/// directories (paths ending with `/`). Returns at most `max_results`
/// matches (default 100 if 0).
pub async fn search_files_by_name(
    workspace_dir: &str,
    query: &str,
    case_sensitive: bool,
    max_results: i32,
) -> Result<Vec<FileNameMatch>, std::io::Error> {
    let all_files = tree::list_files(workspace_dir, ".", true).await?;

    let max = if max_results <= 0 {
        100
    } else {
        max_results as usize
    };
    let query_normalized = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    let mut results = Vec::new();

    for file_path in &all_files {
        // Skip directories
        if file_path.ends_with('/') {
            continue;
        }

        let search_path = if case_sensitive {
            file_path.clone()
        } else {
            file_path.to_lowercase()
        };

        if search_path.contains(&query_normalized) {
            results.push(FileNameMatch {
                path: file_path.clone(),
                matches: 1,
            });
            if results.len() >= max {
                break;
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn search_finds_matching_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("foo.rs"), "").unwrap();
        std::fs::write(tmp.path().join("bar.rs"), "").unwrap();
        std::fs::write(tmp.path().join("baz.txt"), "").unwrap();

        let results = search_files_by_name(tmp.path().to_str().unwrap(), ".rs", false, 0)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn search_respects_max_results() {
        let tmp = TempDir::new().unwrap();
        for i in 0..10 {
            std::fs::write(tmp.path().join(format!("file{i}.txt")), "").unwrap();
        }

        let results = search_files_by_name(tmp.path().to_str().unwrap(), "file", false, 3)
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn search_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("README.md"), "").unwrap();
        std::fs::write(tmp.path().join("readme.txt"), "").unwrap();

        let results = search_files_by_name(tmp.path().to_str().unwrap(), "readme", false, 0)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn search_case_sensitive() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("README.md"), "").unwrap();
        std::fs::write(tmp.path().join("readme.txt"), "").unwrap();

        let results = search_files_by_name(tmp.path().to_str().unwrap(), "README", true, 0)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "README.md");
    }

    #[tokio::test]
    async fn search_skips_directories() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/main.rs"), "").unwrap();

        let results = search_files_by_name(tmp.path().to_str().unwrap(), "src", false, 0)
            .await
            .unwrap();
        // Only src/main.rs should match (contains "src" in path), not the directory itself
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "src/main.rs");
    }
}

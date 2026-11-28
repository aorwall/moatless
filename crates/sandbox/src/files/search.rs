//! File name search within the sandbox workspace.

use std::path::PathBuf;

use ignore::WalkBuilder;

/// A file name match result. `matches` carries the ranking score so the
/// frontend can order/highlight results.
#[derive(Debug, Clone)]
pub struct FileNameMatch {
    pub path: String,
    pub matches: i32,
}

const DEFAULT_MAX_RESULTS: usize = 100;
const HARD_MAX_RESULTS: usize = 500;

/// Search for files by substring match on the file name or path.
///
/// Walks with the `ignore` crate (respects `.gitignore`, skips hidden dirs) on
/// a blocking thread pool, ranks hits (exact > prefix > contains >
/// path-contains), and caps at `max_results`.
pub async fn search_files_by_name(
    workspace_dir: &str,
    query: &str,
    case_sensitive: bool,
    max_results: i32,
) -> Result<Vec<FileNameMatch>, std::io::Error> {
    let workspace = PathBuf::from(workspace_dir);
    let query = query.to_string();

    let max = match max_results {
        n if n <= 0 => DEFAULT_MAX_RESULTS,
        n => usize::try_from(n).map_or(HARD_MAX_RESULTS, |n| n.min(HARD_MAX_RESULTS)),
    };

    tokio::task::spawn_blocking(move || walk_and_rank(&workspace, &query, case_sensitive, max))
        .await
        .map_err(|e| std::io::Error::other(format!("search join failed: {e}")))
}

fn walk_and_rank(
    workspace: &std::path::Path,
    query: &str,
    case_sensitive: bool,
    max: usize,
) -> Vec<FileNameMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_norm = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    let walker = WalkBuilder::new(workspace)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .build();

    let mut scored: Vec<(i32, usize, String)> = Vec::new();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let rel = match entry.path().strip_prefix(workspace) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().to_string();
        if rel_str.is_empty() {
            continue;
        }

        let filename = rel
            .file_name()
            .map_or_default(|s| s.to_string_lossy().to_string());

        let (path_cmp, filename_cmp) = if case_sensitive {
            (rel_str.clone(), filename.clone())
        } else {
            (rel_str.to_lowercase(), filename.to_lowercase())
        };

        let score = if filename_cmp == query_norm {
            1000
        } else if filename_cmp.starts_with(&query_norm) {
            500
        } else if filename_cmp.contains(&query_norm) {
            200
        } else if path_cmp.contains(&query_norm) {
            50
        } else {
            continue;
        };

        scored.push((score, rel_str.len(), rel_str));

        // Early exit: once we have ~10x the cap, stop walking. Ranking
        // still picks the best ones. Keeps walks bounded on huge repos.
        if scored.len() >= max.saturating_mul(10) {
            break;
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    scored
        .into_iter()
        .take(max)
        .map(|(score, _, path)| FileNameMatch {
            path,
            matches: score,
        })
        .collect()
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
    async fn search_hard_ceiling_caps_max_results() {
        let tmp = TempDir::new().unwrap();
        for i in 0..5 {
            std::fs::write(tmp.path().join(format!("file{i}.txt")), "").unwrap();
        }

        // Requesting more than HARD_MAX_RESULTS is clamped.
        let results = search_files_by_name(tmp.path().to_str().unwrap(), "file", false, 10_000)
            .await
            .unwrap();
        assert!(results.len() <= HARD_MAX_RESULTS);
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
        // Only src/main.rs matches via path-contains; the directory itself is skipped.
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "src/main.rs");
    }

    #[tokio::test]
    async fn search_respects_gitignore() {
        let tmp = TempDir::new().unwrap();
        // Fake a git repo so gitignore kicks in.
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "ignored.txt\ntarget/\n").unwrap();
        std::fs::write(tmp.path().join("kept.txt"), "").unwrap();
        std::fs::write(tmp.path().join("ignored.txt"), "").unwrap();
        std::fs::create_dir(tmp.path().join("target")).unwrap();
        std::fs::write(tmp.path().join("target/build.log"), "").unwrap();

        let results = search_files_by_name(tmp.path().to_str().unwrap(), "txt", false, 0)
            .await
            .unwrap();
        let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"kept.txt"));
        assert!(!paths.iter().any(|p| p.contains("ignored.txt")));
        assert!(!paths.iter().any(|p| p.contains("target/")));
    }

    #[tokio::test]
    async fn search_ranks_filename_hits_above_path_hits() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("lib")).unwrap();
        std::fs::write(tmp.path().join("main.rs"), "").unwrap();
        std::fs::write(tmp.path().join("lib/embedded-main-helper.rs"), "").unwrap();

        let results = search_files_by_name(tmp.path().to_str().unwrap(), "main", false, 0)
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].path, "main.rs");
        assert!(results[0].matches > results[1].matches);
    }

    #[tokio::test]
    async fn empty_query_returns_empty() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("file.txt"), "").unwrap();

        let results = search_files_by_name(tmp.path().to_str().unwrap(), "", false, 0)
            .await
            .unwrap();
        assert!(results.is_empty());
    }
}

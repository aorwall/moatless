//! File tree building with optional git change overlay.
//!
//! Ports the TypeScript `file-tree-builder.ts` and `FileService.listFiles()` /
//! `FileService._computeTree()` logic.

use std::collections::HashMap;

use sandbox_proto::UNBROWSABLE_DIRS;
use tokio::fs;

use super::git::{self, GitFileInfo};
use super::{FileError, validate_path};

/// `.`/empty mean the workspace root; anything else goes through
/// [`validate_path`].
///
/// Plain `Path::join` isn't enough: it *replaces* the base on an absolute
/// argument, so `path=/etc` would otherwise walk the container filesystem.
fn resolve_root(workspace_dir: &str, relative_path: &str) -> Result<std::path::PathBuf, FileError> {
    if relative_path == "." || relative_path.is_empty() {
        return Ok(std::path::PathBuf::from(workspace_dir));
    }
    validate_path(workspace_dir, relative_path)
}

/// A node in the file tree (file or directory).
#[derive(Debug, Clone)]
pub struct FileTreeNode {
    pub name: String,
    pub path: String,
    pub node_type: FileNodeType,
    pub is_changed: bool,
    pub git_status: Option<String>,
    pub additions: Option<i32>,
    pub deletions: Option<i32>,
    pub is_staged: Option<bool>,
    pub children: Vec<FileTreeNode>,
}

/// Whether a tree node is a file or directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileNodeType {
    File,
    Directory,
}

/// Summary of git changes across the workspace.
#[derive(Debug, Clone, Default)]
pub struct GitSummary {
    pub total_files: i32,
    pub total_additions: i32,
    pub total_deletions: i32,
    pub total_staged: i32,
    pub total_modified: i32,
    pub compare_base: String,
    pub has_changes: bool,
}

// ─── Directory walking ──────────────────────────────────────────

/// `recursive` walks subdirectories (dirs get a trailing `/`,
/// `.git`/`node_modules` skipped).
///
/// `respect_gitignore` additionally drops what repos ignore — off by default so
/// builds stay visible, on to present the workspace as a repo.
pub async fn list_files(
    workspace_dir: &str,
    relative_path: &str,
    recursive: bool,
    respect_gitignore: bool,
) -> Result<Vec<String>, FileError> {
    let full_path = resolve_root(workspace_dir, relative_path)?;

    let base_prefix = if relative_path == "." || relative_path.is_empty() {
        String::new()
    } else {
        let mut p = relative_path.to_string();
        if !p.ends_with('/') {
            p.push('/');
        }
        p
    };

    if respect_gitignore {
        // The `ignore` walker is synchronous and reads a `.gitignore` per
        // directory it descends into, so it runs off the async workers — the
        // same boundary `search_files_by_name` puts it behind.
        return Ok(tokio::task::spawn_blocking(move || {
            walk_respecting_ignores(&full_path, &base_prefix, recursive)
        })
        .await
        .map_err(|e| std::io::Error::other(format!("list join failed: {e}")))??);
    }

    let mut files = Vec::new();
    walk_dir(&full_path, &base_prefix, recursive, &mut files).await?;
    files.sort();
    Ok(files)
}

/// The same listing as [`walk_dir`], with each repository's ignore rules
/// applied.
///
/// Both yield the same strings for a shared path — a directory with a trailing
/// slash, a file without, relative to and prefixed by the walk's root.
fn walk_respecting_ignores(
    root: &std::path::Path,
    base_prefix: &str,
    recursive: bool,
) -> Result<Vec<String>, std::io::Error> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        // Dotfiles are workspace content — `.github/`, `.cargo/config.toml`.
        // The two directories that are not are skipped by name below, exactly
        // as the plain walk skips them; `.git` in particular is not hidden
        // enough for this walker to leave alone on its own.
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .filter_entry(|entry| !UNBROWSABLE_DIRS.contains(&&*entry.file_name().to_string_lossy()));
    if !recursive {
        builder.max_depth(Some(1));
    }

    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(|e| std::io::Error::other(format!("walk failed: {e}")))?;
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy();
        // The walker yields its own root first, as the empty relative path.
        if relative.is_empty() {
            continue;
        }
        let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
        files.push(if is_dir {
            format!("{base_prefix}{relative}/")
        } else {
            format!("{base_prefix}{relative}")
        });
    }
    files.sort();
    Ok(files)
}

fn walk_dir<'a>(
    dir: &'a std::path::Path,
    base_path: &'a str,
    recursive: bool,
    files: &'a mut Vec<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), std::io::Error>> + Send + 'a>> {
    Box::pin(async move {
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();

            if UNBROWSABLE_DIRS.contains(&name.as_str()) {
                continue;
            }

            let relative = if base_path.is_empty() {
                name.clone()
            } else {
                format!("{base_path}{name}")
            };

            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                files.push(format!("{relative}/"));
                if recursive {
                    let child_base = format!("{relative}/");
                    walk_dir(&entry.path(), &child_base, true, files).await?;
                }
            } else {
                files.push(relative);
            }
        }

        Ok(())
    })
}

// ─── Tree building ──────────────────────────────────────────────

/// Build a hierarchical tree from a flat list of file paths.
///
/// Paths ending in `/` are treated as directories; `git_file_map` provides
/// optional git change overlay.
pub fn build_file_tree(
    file_paths: &[String],
    git_file_map: &HashMap<String, GitFileInfo>,
) -> Vec<FileTreeNode> {
    let mut root: Vec<FileTreeNode> = Vec::new();
    let mut node_map: HashMap<String, usize> = HashMap::new();

    let mut sorted = file_paths.to_vec();
    sorted.sort();

    for file_path in &sorted {
        process_file_path(file_path, &mut root, &mut node_map, git_file_map);
    }

    propagate_changes(&mut root);
    sort_nodes(&mut root);
    root
}

fn process_file_path(
    file_path: &str,
    root: &mut Vec<FileTreeNode>,
    node_map: &mut HashMap<String, usize>,
    git_file_map: &HashMap<String, GitFileInfo>,
) {
    let normalized = file_path.strip_suffix('/').unwrap_or(file_path);
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    let mut current_path = String::new();

    for (i, part) in parts.iter().enumerate() {
        current_path = if current_path.is_empty() {
            part.to_string()
        } else {
            format!("{current_path}/{part}")
        };

        let is_last = i == parts.len() - 1;
        let is_directory = if is_last {
            file_path.ends_with('/')
        } else {
            true
        };

        if node_map.contains_key(&current_path) {
            continue;
        }

        let git_info = git_file_map.get(&current_path);

        let node = FileTreeNode {
            name: part.to_string(),
            path: current_path.clone(),
            node_type: if is_directory {
                FileNodeType::Directory
            } else {
                FileNodeType::File
            },
            is_changed: git_info.is_some(),
            git_status: git_info.map(|g| g.status.clone()),
            additions: git_info.map(|g| g.additions),
            deletions: git_info.map(|g| g.deletions),
            is_staged: git_info.map(|g| g.is_staged),
            children: Vec::new(),
        };

        let parent_path = if i > 0 {
            let parent_parts = &parts[..i];
            Some(parent_parts.join("/"))
        } else {
            None
        };

        if let Some(parent_path) = parent_path {
            // We need to find the parent and add to its children.
            // Since node_map stores indices into a nested structure, we
            // use a flat path → (parent_indices_path) approach.
            // For simplicity, we walk from root using the path components.
            let parent_parts: Vec<&str> = parent_path.split('/').collect();
            if let Some(parent_node) = find_node_mut(root, &parent_parts) {
                let idx = parent_node.children.len();
                parent_node.children.push(node);
                // Store a sentinel — we only use node_map for existence checks
                node_map.insert(current_path.clone(), idx);
            }
        } else {
            let idx = root.len();
            root.push(node);
            node_map.insert(current_path.clone(), idx);
        }
    }
}

/// Walk through a tree to find a node by path components.
fn find_node_mut<'a>(
    nodes: &'a mut [FileTreeNode],
    path_parts: &[&str],
) -> Option<&'a mut FileTreeNode> {
    if path_parts.is_empty() {
        return None;
    }

    let target_name = path_parts[0];
    let node = nodes.iter_mut().find(|n| n.name == target_name)?;

    if path_parts.len() == 1 {
        Some(node)
    } else {
        find_node_mut(&mut node.children, &path_parts[1..])
    }
}

/// Propagate `is_changed` from files up to parent directories.
fn propagate_changes(nodes: &mut [FileTreeNode]) -> bool {
    let mut has_changes = false;

    for node in nodes.iter_mut() {
        if !node.children.is_empty() {
            let children_changed = propagate_changes(&mut node.children);
            if children_changed || node.is_changed {
                node.is_changed = true;
                has_changes = true;
            }
        } else if node.is_changed {
            has_changes = true;
        }
    }

    has_changes
}

/// Sort nodes: directories first, then alphabetically by name.
fn sort_nodes(nodes: &mut [FileTreeNode]) {
    nodes.sort_by(|a, b| {
        let type_cmp = match (&a.node_type, &b.node_type) {
            (FileNodeType::Directory, FileNodeType::File) => std::cmp::Ordering::Less,
            (FileNodeType::File, FileNodeType::Directory) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        };
        type_cmp.then_with(|| a.name.cmp(&b.name))
    });

    for node in nodes.iter_mut() {
        if !node.children.is_empty() {
            sort_nodes(&mut node.children);
        }
    }
}

// ─── Orchestration ──────────────────────────────────────────────

/// Returns `(tree, git_summary)`; without git, annotations are dropped and the
/// summary is empty.
///
/// When `recursive` is false, only `root_path`'s direct children are held, but
/// `is_changed` still reflects the full diff, so folders show change badges.
pub async fn get_file_tree_with_changes(
    workspace_dir: &str,
    root_path: &str,
    recursive: bool,
) -> Result<(Vec<FileTreeNode>, GitSummary), FileError> {
    let all_files = list_files(workspace_dir, root_path, recursive, false).await?;
    let git_diff = git::get_git_diff_detailed(workspace_dir).await;

    let mut git_file_map: HashMap<String, GitFileInfo> = HashMap::new();
    let mut total_staged = 0i32;
    let mut total_modified = 0i32;

    for file in &git_diff.files {
        git_file_map.insert(
            file.filename.clone(),
            GitFileInfo {
                status: file.status.clone(),
                additions: file.additions,
                deletions: file.deletions,
                is_staged: file.is_staged,
            },
        );
        if file.is_staged {
            total_staged += 1;
        } else {
            total_modified += 1;
        }
    }

    let mut tree = build_file_tree(&all_files, &git_file_map);

    // When root_path is a subdirectory, list_files returns paths prefixed with
    // the root, so build_file_tree wraps them in the path components. Unwrap
    // those to return only the direct children of root_path.
    let normalized_root = root_path.trim_matches('/');
    if !normalized_root.is_empty() && normalized_root != "." {
        let parts: Vec<&str> = normalized_root
            .split('/')
            .filter(|p| !p.is_empty())
            .collect();
        let mut subtree = tree;
        for part in parts {
            let pos = subtree
                .iter()
                .position(|n| n.name == part && n.node_type == FileNodeType::Directory);
            if let Some(idx) = pos {
                let node = subtree.swap_remove(idx);
                subtree = node.children;
            } else {
                subtree = Vec::new();
                break;
            }
        }
        tree = subtree;
        sort_nodes(&mut tree);
    }

    // In non-recursive mode, child directories have empty `children` so
    // `propagate_changes` cannot infer a change-badge. Mark each directory
    // changed if any path in the diff lives beneath it.
    if !recursive {
        for node in &mut tree {
            if node.node_type == FileNodeType::Directory && !node.is_changed {
                let prefix = format!("{}/", node.path);
                if git_diff
                    .files
                    .iter()
                    .any(|f| f.filename.starts_with(&prefix))
                {
                    node.is_changed = true;
                }
            }
        }
    }

    let summary = GitSummary {
        total_files: i32::try_from(git_diff.files.len()).unwrap_or(i32::MAX),
        total_additions: git_diff.total_additions,
        total_deletions: git_diff.total_deletions,
        total_staged,
        total_modified,
        compare_base: if git_diff.compare_base.is_empty() {
            "HEAD".to_string()
        } else {
            git_diff.compare_base
        },
        has_changes: !git_diff.files.is_empty(),
    };

    Ok((tree, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute root must not escape the workspace. `Path::join` replaces
    /// the base when its argument is absolute, so this listed the container
    /// filesystem — `path=/etc` enumerated `/etc`, exposing names under `.ssh`
    /// via `/home/<user>`.
    #[tokio::test]
    async fn listing_refuses_an_absolute_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = list_files(tmp.path().to_str().unwrap(), "/etc", false, false)
            .await
            .expect_err("an absolute root is refused");
        assert!(matches!(err, FileError::InvalidPath(_)));
    }

    /// The same escape, spelled with `..` instead.
    #[tokio::test]
    async fn listing_refuses_a_parent_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = list_files(tmp.path().to_str().unwrap(), "../..", false, false)
            .await
            .expect_err("a parent traversal is refused");
        assert!(matches!(err, FileError::InvalidPath(_)));
    }

    /// The tree endpoint walks through `list_files`, so it inherits the same
    /// refusal rather than needing its own.
    #[tokio::test]
    async fn the_tree_refuses_an_absolute_root_too() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = get_file_tree_with_changes(tmp.path().to_str().unwrap(), "/etc", false)
            .await
            .expect_err("an absolute root is refused");
        assert!(matches!(err, FileError::InvalidPath(_)));
    }

    /// The roots real callers pass still work: `.` and `""` mean the workspace
    /// root, and a relative subdirectory resolves inside it.
    #[tokio::test]
    async fn listing_still_accepts_the_workspace_root_and_a_subdirectory() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/a.txt"), "x").unwrap();
        let root = tmp.path().to_str().unwrap();

        assert!(list_files(root, ".", false, false).await.is_ok());
        assert!(list_files(root, "", false, false).await.is_ok());
        let sub = list_files(root, "sub", false, false).await.unwrap();
        assert_eq!(sub, vec!["sub/a.txt".to_string()]);
    }

    #[test]
    fn build_empty_tree() {
        let tree = build_file_tree(&[], &HashMap::new());
        assert!(tree.is_empty());
    }

    #[test]
    fn build_flat_files() {
        let paths = vec!["a.txt".to_string(), "b.txt".to_string()];
        let tree = build_file_tree(&paths, &HashMap::new());
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "a.txt");
        assert_eq!(tree[0].node_type, FileNodeType::File);
        assert!(!tree[0].is_changed);
    }

    #[test]
    fn build_nested_tree() {
        let paths = vec![
            "src/".to_string(),
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "README.md".to_string(),
        ];
        let tree = build_file_tree(&paths, &HashMap::new());

        assert_eq!(tree[0].name, "src");
        assert_eq!(tree[0].node_type, FileNodeType::Directory);
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[0].children[0].name, "lib.rs");
        assert_eq!(tree[0].children[1].name, "main.rs");

        assert_eq!(tree[1].name, "README.md");
        assert_eq!(tree[1].node_type, FileNodeType::File);
    }

    #[test]
    fn build_tree_with_git_overlay() {
        let paths = vec!["src/main.rs".to_string(), "src/lib.rs".to_string()];
        let mut git_map = HashMap::new();
        git_map.insert(
            "src/main.rs".to_string(),
            GitFileInfo {
                status: "modified".to_string(),
                additions: 5,
                deletions: 2,
                is_staged: false,
            },
        );

        let tree = build_file_tree(&paths, &git_map);
        assert_eq!(tree[0].name, "src");
        // Directory should have is_changed propagated
        assert!(tree[0].is_changed);

        let main_rs = &tree[0].children[1]; // main.rs after lib.rs alphabetically
        assert!(main_rs.is_changed);
        assert_eq!(main_rs.git_status.as_deref(), Some("modified"));
        assert_eq!(main_rs.additions, Some(5));
        assert_eq!(main_rs.deletions, Some(2));

        let lib_rs = &tree[0].children[0];
        assert!(!lib_rs.is_changed);
    }

    #[test]
    fn directories_sorted_before_files() {
        let paths = vec![
            "z_file.txt".to_string(),
            "a_dir/".to_string(),
            "a_dir/x.txt".to_string(),
            "b_file.txt".to_string(),
        ];
        let tree = build_file_tree(&paths, &HashMap::new());
        assert_eq!(tree[0].name, "a_dir");
        assert_eq!(tree[0].node_type, FileNodeType::Directory);
        assert_eq!(tree[1].name, "b_file.txt");
        assert_eq!(tree[2].name, "z_file.txt");
    }

    /// Both walks, because the ignore-aware one is only a drop-in for the plain
    /// one if it yields the same strings for the paths they agree on.
    #[tokio::test]
    async fn list_files_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b.txt"), "world").unwrap();

        for respect_gitignore in [false, true] {
            // Non-recursive
            let files = list_files(dir.to_str().unwrap(), ".", false, respect_gitignore)
                .await
                .unwrap();
            assert_eq!(
                files,
                vec!["a.txt".to_string(), "sub/".to_string()],
                "respect_gitignore={respect_gitignore}"
            );

            // Recursive
            let files = list_files(dir.to_str().unwrap(), ".", true, respect_gitignore)
                .await
                .unwrap();
            assert_eq!(
                files,
                vec![
                    "a.txt".to_string(),
                    "sub/".to_string(),
                    "sub/b.txt".to_string()
                ],
                "respect_gitignore={respect_gitignore}"
            );
        }
    }

    #[tokio::test]
    async fn list_files_skips_git_and_node_modules() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        std::fs::create_dir(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/config"), "").unwrap();
        std::fs::create_dir(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules/pkg"), "").unwrap();
        std::fs::write(dir.join("index.js"), "").unwrap();

        for respect_gitignore in [false, true] {
            let files = list_files(dir.to_str().unwrap(), ".", true, respect_gitignore)
                .await
                .unwrap();
            assert_eq!(
                files,
                vec!["index.js".to_string()],
                "respect_gitignore={respect_gitignore}"
            );
        }
    }

    /// The listing a person is shown of a checkout that has been built in.
    ///
    /// The flag is what keeps the build tree out; without it the walk returns
    /// the artifacts too, which is right for a caller looking for one — hence a
    /// flag, not the walk's default behavior.
    #[tokio::test]
    async fn list_files_respecting_gitignore_drops_what_the_repository_ignores() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        // A real repository, because ignore rules only apply inside one.
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(dir.join("main.rs"), "").unwrap();
        std::fs::write(dir.join("build.log"), "").unwrap();
        std::fs::create_dir(dir.join("target")).unwrap();
        std::fs::write(dir.join("target/binary"), "").unwrap();

        let files = list_files(dir.to_str().unwrap(), ".", true, true)
            .await
            .unwrap();
        assert_eq!(
            files,
            vec![".gitignore".to_string(), "main.rs".to_string()],
            "an ignored file and a whole ignored directory both stay out"
        );

        let whole = list_files(dir.to_str().unwrap(), ".", true, false)
            .await
            .unwrap();
        assert!(whole.contains(&"target/binary".to_string()));
        assert!(whole.contains(&"build.log".to_string()));
    }

    /// A workspace that is not a repository has no rules to honour, and is
    /// walked whole either way. This is the state a Sandbox is in before its
    /// clone finishes.
    #[tokio::test]
    async fn list_files_respecting_gitignore_outside_a_repository_hides_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join(".gitignore"), "secret.txt\n").unwrap();
        std::fs::write(dir.join("secret.txt"), "").unwrap();

        let files = list_files(dir.to_str().unwrap(), ".", true, true)
            .await
            .unwrap();
        assert_eq!(
            files,
            vec![".gitignore".to_string(), "secret.txt".to_string()]
        );
    }
}

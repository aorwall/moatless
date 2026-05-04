//! File tree building with optional git change overlay.
//!
//! Ports the TypeScript `file-tree-builder.ts` and `FileService.listFiles()`
//! / `FileService._computeTree()` logic.

use std::collections::HashMap;

use tokio::fs;

use super::git::{self, GitFileInfo};

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

/// Skip these directories when walking the file tree.
const SKIP_DIRS: &[&str] = &[".git", "node_modules"];

/// List files in a directory, relative to workspace root.
///
/// When `recursive` is true, walks subdirectories. Directories are
/// suffixed with `/` in the output. Skips `.git` and `node_modules`.
pub async fn list_files(
    workspace_dir: &str,
    relative_path: &str,
    recursive: bool,
) -> Result<Vec<String>, std::io::Error> {
    let workspace = std::path::Path::new(workspace_dir);
    let full_path = if relative_path == "." || relative_path.is_empty() {
        workspace.to_path_buf()
    } else {
        workspace.join(relative_path)
    };

    let base_prefix = if relative_path == "." || relative_path.is_empty() {
        String::new()
    } else {
        let mut p = relative_path.to_string();
        if !p.ends_with('/') {
            p.push('/');
        }
        p
    };

    let mut files = Vec::new();
    walk_dir(&full_path, &base_prefix, recursive, &mut files).await?;
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

            if SKIP_DIRS.contains(&name.as_str()) {
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
/// Paths ending in `/` are treated as directories.
/// `git_file_map` provides optional git change overlay.
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

        // Find parent path
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

/// Get the file tree with git change overlay.
///
/// Returns `(tree, git_summary)`. If git is unavailable, the tree is
/// returned without change annotations and the summary is empty.
///
/// When `recursive` is false, the tree contains only the direct children
/// of `root_path`. Directory `is_changed` flags are computed from the git
/// diff so collapsed folders still show change badges.
pub async fn get_file_tree_with_changes(
    workspace_dir: &str,
    root_path: &str,
    recursive: bool,
) -> Result<(Vec<FileTreeNode>, GitSummary), std::io::Error> {
    let all_files = list_files(workspace_dir, root_path, recursive).await?;
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
        let parts: Vec<&str> = normalized_root.split('/').filter(|p| !p.is_empty()).collect();
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
                if git_diff.files.iter().any(|f| f.filename.starts_with(&prefix)) {
                    node.is_changed = true;
                }
            }
        }
    }

    let summary = GitSummary {
        total_files: git_diff.files.len() as i32,
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

        // Directories first, then files
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

    #[tokio::test]
    async fn list_files_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b.txt"), "world").unwrap();

        // Non-recursive
        let files = list_files(dir.to_str().unwrap(), ".", false).await.unwrap();
        assert!(files.contains(&"a.txt".to_string()));
        assert!(files.contains(&"sub/".to_string()));
        assert!(!files.contains(&"sub/b.txt".to_string()));

        // Recursive
        let files = list_files(dir.to_str().unwrap(), ".", true).await.unwrap();
        assert!(files.contains(&"a.txt".to_string()));
        assert!(files.contains(&"sub/".to_string()));
        assert!(files.contains(&"sub/b.txt".to_string()));
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

        let files = list_files(dir.to_str().unwrap(), ".", true).await.unwrap();
        assert_eq!(files, vec!["index.js".to_string()]);
    }
}

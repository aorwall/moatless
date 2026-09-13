//! Decode sandbox protobuf file responses into HTTP contracts.

use moatless_api_schemas::sandbox::{
    ContentOrigin, FileContentResponse, FileNameMatchDto, FileTreeNodeDto, FileTreeResponse,
    GitSummaryDto, SearchByNameResponse,
};

/// A sandbox file-tree node carried a `FileNodeType` integer the wire contract
/// does not define, so it cannot be classified as a file or a directory.
///
/// Carries the offending value and path: the tree arrives recursively, so the
/// failing node need not be the one the caller converted.
#[derive(Debug, thiserror::Error)]
#[error("unknown FileNodeType {node_type} for file tree node '{path}'")]
pub struct UnknownFileNodeType {
    pub path: String,
    pub node_type: i32,
}

/// Widening a protobuf file-tree node onto the API DTO is partial: `node_type`
/// is an open integer on the wire, and an unrecognized value must not be
/// reported to clients as a file.
pub fn file_tree_node(
    node: sandbox_proto::FileTreeNode,
) -> Result<FileTreeNodeDto, UnknownFileNodeType> {
    let node_type = match sandbox_proto::FileNodeType::try_from(node.node_type) {
        Ok(sandbox_proto::FileNodeType::File) => "file",
        Ok(sandbox_proto::FileNodeType::Directory) => "directory",
        Err(_) => {
            return Err(UnknownFileNodeType {
                path: node.path,
                node_type: node.node_type,
            });
        }
    };

    // A nested failure fails the whole tree — a partially converted tree
    // would silently hide a subtree from the client.
    let children = node
        .children
        .into_iter()
        .map(file_tree_node)
        .collect::<Result<Vec<_>, _>>()?;

    // Directories always report a (possibly empty) child list so clients can
    // tell "expandable but empty" from "not expandable".
    let children = if children.is_empty() && node_type == "file" {
        None
    } else {
        Some(children)
    };

    Ok(FileTreeNodeDto {
        name: node.name,
        path: node.path,
        node_type: node_type.to_string(),
        is_changed: node.is_changed,
        git_status: node.git_status,
        additions: node.additions,
        deletions: node.deletions,
        is_staged: node.is_staged,
        children,
    })
}

pub fn git_summary(s: sandbox_proto::GitSummary) -> GitSummaryDto {
    GitSummaryDto {
        total_files: s.total_files,
        total_additions: s.total_additions,
        total_deletions: s.total_deletions,
        total_staged: s.total_staged,
        total_modified: s.total_modified,
        compare_base: s.compare_base,
        has_changes: s.has_changes,
    }
}

pub fn file_content(r: sandbox_proto::ReadFileResponse) -> FileContentResponse {
    FileContentResponse {
        content: r.content,
        old_content: r.old_content,
        additions: r.additions,
        deletions: r.deletions,
        is_modified: r.is_modified,
        is_binary: r.is_binary,
        media_type: r.media_type,
        origin: ContentOrigin::sandbox(),
    }
}

pub fn file_name_match(m: sandbox_proto::FileNameMatch) -> FileNameMatchDto {
    FileNameMatchDto {
        path: m.path,
        matches: m.matches,
    }
}

pub fn file_tree(
    resp: sandbox_proto::GetFileTreeResponse,
) -> Result<FileTreeResponse, UnknownFileNodeType> {
    let git_summary = resp.git_summary.map_or(
        GitSummaryDto {
            total_files: 0,
            total_additions: 0,
            total_deletions: 0,
            total_staged: 0,
            total_modified: 0,
            compare_base: "HEAD".to_string(),
            has_changes: false,
        },
        git_summary,
    );

    Ok(FileTreeResponse {
        tree: resp
            .tree
            .into_iter()
            .map(file_tree_node)
            .collect::<Result<Vec<_>, _>>()?,
        git_summary,
        origin: ContentOrigin::sandbox(),
    })
}

pub fn search_by_name(resp: sandbox_proto::SearchFilesByNameResponse) -> SearchByNameResponse {
    SearchByNameResponse {
        files: resp.files.into_iter().map(file_name_match).collect(),
        total_matches: resp.total_matches,
        origin: ContentOrigin::sandbox(),
    }
}

#[cfg(test)]
mod file_tree_conversion_tests {
    use super::*;

    fn proto_node(
        path: &str,
        node_type: i32,
        children: Vec<sandbox_proto::FileTreeNode>,
    ) -> sandbox_proto::FileTreeNode {
        sandbox_proto::FileTreeNode {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.to_string(),
            node_type,
            is_changed: false,
            git_status: None,
            additions: None,
            deletions: None,
            is_staged: None,
            children,
        }
    }

    #[test]
    fn every_known_node_type_converts_to_its_dto_string() {
        for (node_type, expected) in [
            (sandbox_proto::FileNodeType::File, "file"),
            (sandbox_proto::FileNodeType::Directory, "directory"),
        ] {
            let dto = file_tree_node(proto_node("a", node_type as i32, vec![]))
                .expect("known node type converts");
            assert_eq!(dto.node_type, expected);
        }
    }

    #[test]
    fn directories_keep_an_empty_child_list_and_files_have_none() {
        let dir = file_tree_node(proto_node(
            "src",
            sandbox_proto::FileNodeType::Directory as i32,
            vec![],
        ))
        .expect("directory converts");
        assert_eq!(dir.children.as_ref().map(Vec::len), Some(0));

        let file = file_tree_node(proto_node(
            "a.txt",
            sandbox_proto::FileNodeType::File as i32,
            vec![],
        ))
        .expect("file converts");
        assert!(file.children.is_none());
    }

    #[test]
    fn unknown_node_type_is_rejected_with_the_offending_value() {
        let err = file_tree_node(proto_node("weird.sock", 7, vec![]))
            .expect_err("unknown node type must not be treated as a file");
        assert_eq!(err.node_type, 7);
        assert_eq!(err.path, "weird.sock");
        assert_eq!(
            err.to_string(),
            "unknown FileNodeType 7 for file tree node 'weird.sock'"
        );
    }

    #[test]
    fn nested_unknown_node_type_propagates_to_the_root_conversion() {
        let deep = proto_node(
            "src",
            sandbox_proto::FileNodeType::Directory as i32,
            vec![proto_node(
                "src/nested",
                sandbox_proto::FileNodeType::Directory as i32,
                vec![
                    proto_node(
                        "src/nested/ok.rs",
                        sandbox_proto::FileNodeType::File as i32,
                        vec![],
                    ),
                    proto_node("src/nested/mystery", 42, vec![]),
                ],
            )],
        );

        let err = file_tree_node(deep).expect_err("a nested failure must fail the whole tree");
        // The error names the failing descendant, not the node converted.
        assert_eq!(err.path, "src/nested/mystery");
        assert_eq!(err.node_type, 42);
    }

    #[test]
    fn response_conversion_propagates_nested_failures() {
        let resp = sandbox_proto::GetFileTreeResponse {
            tree: vec![proto_node(
                "src",
                sandbox_proto::FileNodeType::Directory as i32,
                vec![proto_node("src/mystery", -1, vec![])],
            )],
            git_summary: None,
        };

        let err = file_tree(resp).expect_err("nested failure fails the response");
        assert_eq!(err.path, "src/mystery");
        assert_eq!(err.node_type, -1);
    }

    #[test]
    fn valid_tree_converts_with_children_and_git_overlay_intact() {
        let resp = sandbox_proto::GetFileTreeResponse {
            tree: vec![sandbox_proto::FileTreeNode {
                is_changed: true,
                git_status: Some("modified".to_string()),
                additions: Some(3),
                deletions: Some(1),
                is_staged: Some(false),
                ..proto_node(
                    "src",
                    sandbox_proto::FileNodeType::Directory as i32,
                    vec![proto_node(
                        "src/main.rs",
                        sandbox_proto::FileNodeType::File as i32,
                        vec![],
                    )],
                )
            }],
            git_summary: None,
        };

        let dto = file_tree(resp).expect("valid tree converts");
        let root = &dto.tree[0];
        assert_eq!(root.node_type, "directory");
        assert!(root.is_changed);
        assert_eq!(root.git_status.as_deref(), Some("modified"));
        assert_eq!(root.additions, Some(3));
        assert_eq!(root.deletions, Some(1));
        assert_eq!(root.is_staged, Some(false));
        let children = root.children.as_ref().expect("directory has children");
        assert_eq!(children[0].path, "src/main.rs");
        assert_eq!(children[0].node_type, "file");
        // Absent git summary still yields the documented default.
        assert_eq!(dto.git_summary.compare_base, "HEAD");
        assert!(!dto.git_summary.has_changes);
    }
}

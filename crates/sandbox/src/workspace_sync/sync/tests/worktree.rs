use super::*;

/// `ws/repo` with `target/` ignored and one commit, plus a linked worktree of
/// it at `ws/wt` holding ignored build output.
fn workspace_with_worktree(ws: &std::path::Path) -> std::path::PathBuf {
    let repo = ws.join("repo");
    init_repo(&repo);
    std::fs::write(repo.join(".gitignore"), b"target/\n").unwrap();
    std::fs::write(repo.join("main.rs"), b"fn main() {}\n").unwrap();
    git_add(&repo, &["main.rs", ".gitignore"]);
    git_commit(&repo, "init");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "-q", "--detach", "../wt"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let wt = ws.join("wt");
    std::fs::create_dir_all(wt.join("target")).unwrap();
    std::fs::write(wt.join("target/big.bin"), b"build output").unwrap();
    wt
}

#[tokio::test]
async fn fingerprint_ignores_build_churn_in_a_worktree() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let wt = workspace_with_worktree(&ws);

    let base = workspace_fingerprint(ws.clone())
        .await
        .expect("workspace has content");

    std::fs::write(wt.join("target/big.bin"), b"more build output").unwrap();
    assert_eq!(base, workspace_fingerprint(ws.clone()).await.unwrap());

    std::fs::write(wt.join("main.rs"), b"fn main() { work(); }\n").unwrap();
    assert_ne!(base, workspace_fingerprint(ws.clone()).await.unwrap());
}

#[test]
fn a_worktree_honors_its_gitignore_in_the_tar() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("workspace");
    workspace_with_worktree(&ws);

    let mut builder = tar::Builder::new(Vec::new());
    assert!(append_tree(&mut builder, &ws, &ws));
    let archived = builder.into_inner().unwrap();

    let mut archive = tar::Archive::new(&archived[..]);
    let names: Vec<String> = archive
        .entries()
        .unwrap()
        .flatten()
        .map(|entry| entry.path().unwrap().display().to_string())
        .collect();
    assert!(names.iter().any(|name| name == "wt/.git"), "{names:?}");
    assert!(names.iter().any(|name| name == "wt/main.rs"), "{names:?}");
    assert!(
        !names.iter().any(|name| name.starts_with("wt/target")),
        "{names:?}"
    );
}

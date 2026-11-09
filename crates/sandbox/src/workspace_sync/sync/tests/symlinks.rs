use super::*;

/// A venv's `bin/python` and `node_modules/.bin/bun` sit outside any repo.
#[tokio::test]
async fn a_symlink_outside_a_repo_survives_a_snapshot_round_trip() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let bin = ws.join("scratch/venv/bin");
    tokio::fs::create_dir_all(&bin).await.unwrap();
    std::os::unix::fs::symlink("/usr/bin/python3", bin.join("python")).unwrap();
    let before = workspace_fingerprint(ws.clone()).await;

    let tar = build_tar(ws.clone()).await.expect("tar built");
    let dst = tmp.path().join("restored-ws");
    tokio::fs::create_dir_all(&dst).await.unwrap();
    extract_workspace(tar, dst.clone(), dst.join("none"), String::new(), true)
        .await
        .unwrap();

    let link = dst.join("scratch/venv/bin/python");
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        Path::new("/usr/bin/python3")
    );
    std::fs::remove_file(bin.join("python")).unwrap();
    assert_ne!(
        workspace_fingerprint(ws).await,
        before,
        "a removed symlink must change the fingerprint"
    );
}

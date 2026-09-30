//! Write the GitHub token straight into `~/.config/gh/hosts.yml`.
//!
//! `gh auth git-credential` reads this file fresh on every invocation, so
//! writing here each refresh cycle lets git/gh pick up rotation without an env
//! change. We skip `gh auth login --with-token`, which hard-requires `read:org`
//! and rejects narrower tokens that are otherwise sufficient for `git push`.
//! This is the only writer of `hosts.yml`; it renders the whole file rather
//! than merging.

use std::path::PathBuf;

use moatless_types::http::HttpTimeouts;
use tracing::{debug, warn};

/// Username recorded when the token's own login can't be resolved.
///
/// Matches what git itself uses for GitHub App installation tokens (`ghs_`,
/// which get 403 from `GET /user`); `gh`'s multi-account migration requires
/// some user key even though it only reads the token back out.
const FALLBACK_LOGIN: &str = "x-access-token";

/// Write `token` into the gh CLI's `hosts.yml`.
///
/// `home_dir` locates the file. Idempotent — safe to call every refresh cycle.
/// `github` bounds the `GET /user` lookup that resolves the token's login.
pub async fn write_github_token(
    home_dir: &str,
    token: &str,
    github: HttpTimeouts,
) -> Result<(), String> {
    let login = resolve_login(token, github).await.unwrap_or_else(|| {
        debug!("Could not resolve GitHub login for token; recording it as {FALLBACK_LOGIN}");
        FALLBACK_LOGIN.to_string()
    });

    write_hosts_file(home_dir, &login, token).await?;

    debug!(login, "Refreshed GitHub token in gh hosts.yml");
    Ok(())
}

/// Render and write the file. Split from [`write_github_token`] so the on-disk
/// behaviour is testable without the network round-trip that resolves `login`.
async fn write_hosts_file(home_dir: &str, login: &str, token: &str) -> Result<(), String> {
    let path = hosts_path(home_dir);
    let dir = path
        .parent()
        .ok_or_else(|| format!("gh hosts path has no parent: {}", path.display()))?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;

    write_private(&path, &render_hosts_yml(login, token)).await
}

fn hosts_path(home_dir: &str) -> PathBuf {
    PathBuf::from(home_dir)
        .join(".config")
        .join("gh")
        .join("hosts.yml")
}

/// Render the single-host `hosts.yml` `gh` expects.
///
/// Writes both the top-level `oauth_token` (older `gh`) and the `users` entry
/// (multi-account migration), so neither version has to migrate the file.
fn render_hosts_yml(login: &str, token: &str) -> String {
    format!(
        "github.com:\n    \
         users:\n        \
         {login}:\n            \
         oauth_token: {token}\n    \
         oauth_token: {token}\n    \
         user: {login}\n    \
         git_protocol: https\n"
    )
}

/// Write `contents` to `path` at `0600` via an atomic same-dir rename, so a
/// racing `gh` invocation never sees a partial file.
///
/// The temp file is *created* `0600` (never chmod'd, which would leave a window
/// at the umask's mode) and opened via `create_new` after an unlink, so a
/// leftover file — wrong permissions, or a symlink — can't be written through.
async fn write_private(path: &std::path::Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("yml.tmp");

    // Best-effort: the create below is what actually guarantees a fresh inode.
    let _ = tokio::fs::remove_file(&tmp).await;

    if let Err(e) = write_new_private(&tmp, contents).await {
        // Don't leave a half-written token lying around under any name.
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!("Failed to write {}: {e}", tmp.display()));
    }

    tokio::fs::rename(&tmp, path).await.map_err(|e| {
        format!(
            "Failed to move {} into place at {}: {e}",
            tmp.display(),
            path.display()
        )
    })
}

/// Create `path` owner-only and write `contents` into it. Fails if it exists.
async fn write_new_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);

    // `tokio::fs::OpenOptions` exposes `mode` inherently on unix.
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options.open(path).await?;
    file.write_all(contents.as_bytes()).await?;
    file.flush().await
}

/// Resolve the GitHub login the token authenticates as.
///
/// Returns `None` on any failure — a wrong username in `hosts.yml` is cosmetic
/// (`gh` reads the token back out regardless), never a reason to fail.
#[expect(
    clippy::disallowed_methods,
    reason = "sets the timeouts `Config::http` holds"
)]
async fn resolve_login(token: &str, github: HttpTimeouts) -> Option<String> {
    let client = reqwest::Client::builder()
        .connect_timeout(github.connect)
        .timeout(github.total)
        .build()
        .map_err(|e| warn!(error = %e, "Failed to build HTTP client for GitHub login lookup"))
        .ok()?;

    let response = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "moatless-sandbox")
        .send()
        .await
        .map_err(|e| debug!(error = %e, "GitHub login lookup failed"))
        .ok()?;

    if !response.status().is_success() {
        debug!(status = %response.status(), "GitHub login lookup returned non-success");
        return None;
    }

    // `reqwest` is built without its `json` feature here, so parse the body
    // ourselves rather than widening the dependency for one field.
    let body = response
        .text()
        .await
        .map_err(|e| debug!(error = %e, "GitHub login lookup returned an unreadable body"))
        .ok()?;

    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| debug!(error = %e, "GitHub login lookup returned unparseable body"))
        .ok()?;

    parsed.get("login")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_both_token_locations() {
        let yml = render_hosts_yml("octocat", "ghp_secret");

        // Legacy readers take the top-level key, multi-account ones the nested
        // user entry. Missing either forces gh to migrate the file on first use.
        assert!(yml.contains("\n    oauth_token: ghp_secret\n"));
        assert!(yml.contains("\n            oauth_token: ghp_secret\n"));
        assert!(yml.contains("\n        octocat:\n"));
        assert!(yml.contains("\n    user: octocat\n"));
        assert!(yml.starts_with("github.com:\n"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writes_hosts_file_readable_only_by_owner() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().display().to_string();

        write_hosts_file(&home, "octocat", "ghp_secret")
            .await
            .unwrap();

        let path = hosts_path(&home);
        let meta = tokio::fs::metadata(&path).await.unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("ghp_secret"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn creates_the_config_directory_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().join("fresh-home");
        let home = home.display().to_string();

        write_hosts_file(&home, "octocat", "ghp_secret")
            .await
            .unwrap();

        assert!(hosts_path(&home).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn overwrites_a_stale_token_and_leaves_no_temp_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().display().to_string();

        write_hosts_file(&home, "octocat", "ghp_stale")
            .await
            .unwrap();
        write_hosts_file(&home, "octocat", "ghp_fresh")
            .await
            .unwrap();

        let path = hosts_path(&home);
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("ghp_fresh"));
        assert!(!contents.contains("ghp_stale"));

        // A leftover .tmp would be a second copy of the token on disk.
        assert!(!path.with_extension("yml.tmp").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn temp_file_carries_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let scratch = tmp.path().join("hosts.yml.tmp");

        write_new_private(&scratch, "github.com:\n").await.unwrap();

        let meta = tokio::fs::metadata(&scratch).await.unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_to_write_through_a_leftover_temp_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let scratch = tmp.path().join("hosts.yml.tmp");
        tokio::fs::write(&scratch, "stale").await.unwrap();

        // This is the test that pins *creation-time* privacy. Asserting the
        // mode after the fact can't distinguish create-0600 from
        // write-then-chmod — both end at 0600, and the second is only unsafe
        // for the window in between. Refusing an existing inode can only hold
        // for `create_new`, which is the same call that carries the mode, so
        // this fails the moment someone reintroduces a write-first version.
        // A leftover inode may also carry permissions we never set, or be a
        // symlink aimed elsewhere.
        let err = write_new_private(&scratch, "fresh").await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);

        // write_private unlinks first, so the full path still succeeds.
        write_private(&tmp.path().join("hosts.yml"), "fresh")
            .await
            .unwrap();
        let contents = tokio::fs::read_to_string(tmp.path().join("hosts.yml"))
            .await
            .unwrap();
        assert_eq!(contents, "fresh");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fallback_login_still_produces_a_user_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().display().to_string();

        write_hosts_file(&home, FALLBACK_LOGIN, "ghs_installation")
            .await
            .unwrap();

        let contents = tokio::fs::read_to_string(hosts_path(&home)).await.unwrap();
        // gh's multi-account migration errors on a host entry with no user, so
        // an unresolved login must still produce one.
        assert!(contents.contains(&format!("user: {FALLBACK_LOGIN}")));
        assert!(contents.contains(&format!("        {FALLBACK_LOGIN}:\n")));
    }
}

//! Write the GitHub token straight into `~/.config/gh/hosts.yml`.
//!
//! The `gh` CLI's git credential helper (`gh auth git-credential`) reads this
//! file fresh on every invocation. By writing the current token here on each
//! refresh cycle, git/gh operations spawned from a long-running agent turn pick
//! up rotated tokens without needing the agent's process env to change.
//!
//! # Why not `gh auth login --with-token`
//!
//! That command validates the token before persisting it and **hard-requires
//! the `read:org` scope**, exiting non-zero on any token that lacks it. The
//! credential the sandbox is handed is frequently narrower than that — a
//! workspace-pinned PAT is meant to be a least-privilege machine credential,
//! and `repo` alone is enough for everything the sandbox does with git. Routing
//! through `gh auth login` made `read:org` a de-facto requirement for using
//! `gh` at all: without it login failed, no `hosts.yml` was written, and `gh`
//! sat unauthenticated even though the very same token drove `git push` fine
//! through the credential broker.
//!
//! Writing the file ourselves keeps the scope requirement where it belongs — on
//! the API call that actually needs the scope, which fails with GitHub's own
//! error rather than being pre-empted by a blanket check.
//!
//! This function is the only writer of `hosts.yml` in the pod, so it renders
//! the whole file rather than merging. A host entry added by hand inside the
//! sandbox does not survive the next refresh cycle.

use std::path::PathBuf;
use std::time::Duration;

use tracing::{debug, warn};

/// Ceiling on the `GET /user` lookup that resolves the token's login.
const LOGIN_LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Username recorded when the token's own login can't be resolved.
///
/// GitHub App installation tokens (`ghs_`) have no user behind them and answer
/// `GET /user` with 403; this is the username git itself uses for them, so it
/// stays truthful about what the credential is. `gh` needs *some* user key —
/// its multi-account migration errors out on a host entry without one — but
/// only ever reads the token back out.
const FALLBACK_LOGIN: &str = "x-access-token";

/// Write `token` into the gh CLI's `hosts.yml`.
///
/// `home_dir` locates the file. Idempotent — safe to call every refresh cycle.
pub async fn write_github_token(home_dir: &str, token: &str) -> Result<(), String> {
    let login = resolve_login(token).await.unwrap_or_else(|| {
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
/// Both the top-level `oauth_token` and the `users` entry are written: the
/// former is what older `gh` reads directly, the latter is what its
/// multi-account migration expects to find. Emitting both means neither
/// version has to migrate the file on first use.
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

/// Write `contents` to `path` with `0600`, never leaving a partial file where
/// `gh` could read it.
///
/// The token goes to a temp file in the same directory and is then renamed over
/// the target — an atomic swap on the same filesystem, so a `gh` invocation
/// racing the refresher sees either the old file or the new one.
///
/// The temp file is *created* `0600` rather than chmod'd afterwards. Writing
/// first and tightening second would put the token on disk at the umask's mode
/// (`0644` under the usual `022`) for the width of that window, and the temp
/// path is predictable, so anything else in the sandbox could open it in the
/// meantime. `create_new` after an unlink means we own the inode we're writing
/// into: a leftover temp file from an interrupted run — which may carry
/// permissions we never set, or be a symlink aimed elsewhere — can't be
/// written through.
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
/// (`gh` reads the token back out regardless), so this must never be the reason
/// a token fails to reach `gh`.
async fn resolve_login(token: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(LOGIN_LOOKUP_TIMEOUT)
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

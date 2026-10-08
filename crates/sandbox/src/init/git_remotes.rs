//! Keep credentials out of the git URLs whose origin the credential broker
//! answers.
//!
//! A credential in a URL outranks every credential helper, so a token written
//! into `remote.<name>.url` or a `url.<base>.insteadOf` base keeps being sent
//! after it expires. `.git/config` is in the workspace snapshot, so a restored
//! sandbox brings such a URL back; [`scrub_workspace`] runs on every start.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokio::process::Command;
use tracing::{info, warn};

/// How deep under the workspace to look for checkouts: deep enough for an
/// isolated Member checkout at `.moatless/members/<task>/<repo>`.
const MAX_DEPTH: usize = 4;

/// Directories never holding a checkout worth the walk.
const SKIPPED_DIRS: [&str; 2] = ["node_modules", "target"];

/// The `scheme://host[:port]` origin of an `http(s)` URL. `None` for any
/// other URL.
pub(crate) fn url_origin(url: &str) -> Option<String> {
    split_http_url(url).map(|parts| parts.origin)
}

/// `url` without its userinfo, when it has one and its origin is in
/// `broker_origins`. Any other URL keeps its credential: no helper answers its
/// host, so stripping it would leave git with nothing.
pub(crate) fn strip_broker_userinfo(url: &str, broker_origins: &[String]) -> Option<String> {
    let parts = split_http_url(url)?;
    if !parts.has_userinfo
        || !broker_origins
            .iter()
            .any(|origin| origin.eq_ignore_ascii_case(&parts.origin))
    {
        return None;
    }
    Some(parts.plain)
}

struct HttpUrlParts {
    origin: String,
    plain: String,
    has_userinfo: bool,
}

fn split_http_url(url: &str) -> Option<HttpUrlParts> {
    let scheme_len = ["https://", "http://"]
        .into_iter()
        .find(|scheme| {
            url.get(..scheme.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
        })?
        .len();
    let (scheme, rest) = url.split_at(scheme_len);
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let (has_userinfo, host) = match authority.rsplit_once('@') {
        Some((_, host)) => (true, host),
        None => (false, authority),
    };
    if host.is_empty() {
        return None;
    }
    Some(HttpUrlParts {
        origin: format!("{}{host}", scheme.to_ascii_lowercase()),
        plain: format!("{scheme}{host}{path}"),
        has_userinfo,
    })
}

/// Strip broker-origin credentials from the global git config and from every
/// checkout under `workspace_dir`. Best-effort: a failure is logged and the
/// start goes on.
pub(crate) async fn scrub_workspace(workspace_dir: &Path, broker_origins: &[String]) {
    if broker_origins.is_empty() {
        return;
    }
    let root = workspace_dir.to_path_buf();
    let checkouts = tokio::task::spawn_blocking(move || find_checkouts(&root))
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Finding checkouts to scrub panicked");
            Vec::new()
        });

    let mut scrubbed = scrub_config(&ConfigScope::Global, broker_origins).await;
    for checkout in checkouts {
        scrubbed += scrub_config(&ConfigScope::Local(checkout), broker_origins).await;
    }
    if scrubbed > 0 {
        info!(
            entries = scrubbed,
            "Removed credentials from git remote URLs"
        );
    }
}

/// Every directory holding a `.git` within [`MAX_DEPTH`] of `root`, not
/// descending into a checkout or through a symlink.
fn find_checkouts(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = pending.pop() {
        if dir.join(".git").exists() {
            found.push(dir);
            continue;
        }
        if depth == MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let is_dir = entry.file_type().is_ok_and(|ft| ft.is_dir());
            let skipped = SKIPPED_DIRS.iter().any(|name| entry.file_name() == *name);
            if is_dir && !skipped {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    found
}

enum ConfigScope {
    Global,
    Local(PathBuf),
}

/// One scope's credential-bearing entries rewritten; returns how many.
async fn scrub_config(scope: &ConfigScope, broker_origins: &[String]) -> usize {
    let entries = list_entries(scope).await;
    let mut scrubbed = 0;

    // A remote keeps its keys; only the values change.
    let mut remote_values: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    // A rewrite's base is its section name, so the section itself has to go.
    let mut url_sections: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for (key, value) in &entries {
        if key.starts_with("remote.") {
            remote_values.entry(key).or_default().push(value);
        } else if let Some((base, variable)) = key
            .strip_prefix("url.")
            .and_then(|rest| rest.rsplit_once('.'))
        {
            url_sections
                .entry(base)
                .or_default()
                .push((variable, value));
        }
    }

    for (key, values) in remote_values {
        if !values
            .iter()
            .any(|value| strip_broker_userinfo(value, broker_origins).is_some())
        {
            continue;
        }
        let mut ok = run_git(scope, &["--unset-all", key]).await;
        for value in &values {
            let plain = strip_broker_userinfo(value, broker_origins);
            scrubbed += usize::from(plain.is_some());
            ok &= run_git(scope, &["--add", key, plain.as_deref().unwrap_or(value)]).await;
        }
        report(ok, scope, key);
    }

    for (base, rewrites) in url_sections {
        let Some(plain) = strip_broker_userinfo(base, broker_origins) else {
            continue;
        };
        let mut ok = run_git(scope, &["--remove-section", &format!("url.{base}")]).await;
        for (variable, value) in &rewrites {
            // A rewrite onto its own plain base does nothing.
            if *value != plain {
                ok &= run_git(scope, &["--add", &format!("url.{plain}.{variable}"), value]).await;
            }
        }
        // The section name holds the credential, so it stays out of the log.
        report(ok, scope, "url.<base>.insteadOf");
        scrubbed += rewrites.len();
    }
    scrubbed
}

fn report(ok: bool, scope: &ConfigScope, key: &str) {
    if !ok {
        let dir = match scope {
            ConfigScope::Global => "global".to_string(),
            ConfigScope::Local(dir) => dir.display().to_string(),
        };
        warn!(scope = %dir, key, "Failed to remove a credential from a git URL");
    }
}

/// Every remote URL and URL rewrite in `scope`, as `(key, value)`.
async fn list_entries(scope: &ConfigScope) -> Vec<(String, String)> {
    let output = git_config(scope)
        .args([
            "-z",
            "--get-regexp",
            r"^(remote\..*\.(url|pushurl)|url\..*\.(insteadof|pushinsteadof))$",
        ])
        .output()
        .await;
    // Exit 1 is "no entry matched".
    let Some(output) = output.ok().filter(|output| output.status.success()) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter_map(|entry| {
            let (key, value) = entry.split_once('\n')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// Run `git config <args>` in `scope`. Logs nothing itself: git's stderr can
/// echo a key that holds a credential.
async fn run_git(scope: &ConfigScope, args: &[&str]) -> bool {
    git_config(scope)
        .args(args)
        .output()
        .await
        .is_ok_and(|output| output.status.success())
}

fn git_config(scope: &ConfigScope) -> Command {
    let mut command = Command::new("git");
    match scope {
        ConfigScope::Global => command.args(["config", "--global"]),
        ConfigScope::Local(dir) => command.arg("-C").arg(dir).args(["config", "--local"]),
    };
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    const GITHUB: &str = "https://github.com";

    fn origins() -> Vec<String> {
        vec![GITHUB.to_string(), "http://backend:8080".to_string()]
    }

    #[test]
    fn only_a_broker_origin_loses_its_userinfo() {
        assert_eq!(
            strip_broker_userinfo("https://x-access-token:ghs_old@github.com/o/r", &origins()),
            Some("https://github.com/o/r".to_string())
        );
        assert_eq!(
            strip_broker_userinfo("http://x:jwt@backend:8080/git/r.git", &origins()),
            Some("http://backend:8080/git/r.git".to_string())
        );
        // Already plain.
        assert_eq!(
            strip_broker_userinfo("https://github.com/o/r", &origins()),
            None
        );
        // No helper answers GitLab, so its token is all git has.
        assert_eq!(
            strip_broker_userinfo("https://oauth2:glpat@gitlab.com/o/r", &origins()),
            None
        );
        assert_eq!(
            strip_broker_userinfo("git@github.com:o/r.git", &origins()),
            None
        );
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// The restored-snapshot shape: a checkout whose `.git/config` carries an
    /// expired token in `origin` and in a URL rewrite.
    #[tokio::test]
    async fn a_restored_config_with_an_embedded_token_is_rewritten() {
        let tmp = tempfile::tempdir().unwrap();
        let primary = tmp.path().join("api");
        let member = tmp.path().join(".moatless/members/t1/api");
        let gitlab = tmp.path().join("infra");
        for dir in [&primary, &member, &gitlab] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "--quiet"]);
        }
        git(
            &primary,
            &[
                "remote",
                "add",
                "origin",
                "https://x-access-token:ghs_old@github.com/o/api.git",
            ],
        );
        git(
            &primary,
            &[
                "config",
                "--add",
                "remote.origin.pushurl",
                "https://ghs_old@github.com/o/api.git",
            ],
        );
        git(
            &primary,
            &[
                "config",
                "--add",
                "url.https://x-access-token:ghs_old@github.com/.insteadOf",
                "https://github.com/",
            ],
        );
        git(
            &primary,
            &[
                "config",
                "--add",
                "url.https://x-access-token:ghs_old@github.com/.insteadOf",
                "git@github.com:",
            ],
        );
        git(
            &member,
            &[
                "remote",
                "add",
                "origin",
                "https://x-access-token:ghs_old@github.com/o/api.git",
            ],
        );
        git(
            &gitlab,
            &[
                "remote",
                "add",
                "origin",
                "https://oauth2:glpat@gitlab.com/o/infra.git",
            ],
        );

        // The global scope is the developer's own here, so only the local ones
        // run.
        let found = find_checkouts(tmp.path());
        assert_eq!(found.len(), 3);
        for dir in found {
            scrub_config(&ConfigScope::Local(dir), &origins()).await;
        }

        let config = std::fs::read_to_string(primary.join(".git/config")).unwrap();
        assert!(!config.contains("ghs_old"), "{config}");
        assert_eq!(
            git(&primary, &["config", "remote.origin.url"]),
            "https://github.com/o/api.git"
        );
        assert_eq!(
            git(&primary, &["config", "remote.origin.pushurl"]),
            "https://github.com/o/api.git"
        );
        // The rewrite onto itself is dropped; the SSH one survives, plain.
        assert_eq!(
            git(
                &primary,
                &[
                    "config",
                    "--local",
                    "--get-all",
                    "url.https://github.com/.insteadOf"
                ]
            ),
            "git@github.com:"
        );
        assert_eq!(
            git(&member, &["config", "remote.origin.url"]),
            "https://github.com/o/api.git"
        );
        assert_eq!(
            git(&gitlab, &["config", "remote.origin.url"]),
            "https://oauth2:glpat@gitlab.com/o/infra.git"
        );
    }

    #[test]
    fn the_walk_stops_at_a_checkout_and_skips_build_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        for dir in ["api/.git", "api/vendor/lib/.git", "web/node_modules/x/.git"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        assert_eq!(find_checkouts(tmp.path()), vec![tmp.path().join("api")]);
    }
}

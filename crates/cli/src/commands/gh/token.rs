//! Which GitHub credential a spawned `gh` gets, and where it comes from.

use std::io::Write;

/// The broker socket `docker/moatless-git-cred.sh` pulls from. Must match the
/// path that script bakes in, and the one `moatless-sandbox` binds.
const BROKER_SOCKET: &str = "/run/moatless/credentials.sock";

/// The socket a sandbox's `gh` sends every request over, which the backend
/// answers with a live token. Must match the one `moatless-sandbox` binds.
const GITHUB_PROXY_SOCKET: &str = "/run/moatless/github.sock";

/// `env_token_set` is [`Config::github_token_set`](crate::config::Config::github_token_set).
pub(crate) fn github_token_for_gh(env_token_set: bool) -> Option<String> {
    resolve_gh_token(
        std::path::Path::new(GITHUB_PROXY_SOCKET).exists(),
        std::path::Path::new(BROKER_SOCKET).exists(),
        env_token_set,
        || git_credential_password("github.com").ok(),
    )
}

/// Pick the token to hand `gh`, or `None` to let `gh` resolve its own.
///
/// Behind the GitHub proxy `gh` needs none: its config sends every request over
/// the socket, and the backend supplies the token. With only the broker the
/// credential helper wins over `GH_TOKEN`, a provision-time snapshot that 401s
/// once its installation token expires. Off both an exported `GH_TOKEN` wins.
fn resolve_gh_token(
    proxy_present: bool,
    broker_present: bool,
    env_token_set: bool,
    credential: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if proxy_present || (env_token_set && !broker_present) {
        return None;
    }
    credential()
}

fn git_credential_password(host: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut child = std::process::Command::new("git")
        .args(["credential", "fill"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to run git credential fill: {e}"))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or("failed to open git credential fill stdin")?;
        write!(stdin, "protocol=https\nhost={host}\n\n")?;
    }

    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err("git credential fill did not return a GitHub token".into());
    }

    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("password="))
        .map(str::trim)
        .filter(|password| !password.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "git credential fill did not return a password".into())
}

#[cfg(test)]
mod tests;

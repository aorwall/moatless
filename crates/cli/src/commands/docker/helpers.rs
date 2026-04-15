//! Shared Docker / docker compose subprocess helpers.
//!
//! Centralises binary detection, compose-dir resolution, `.env` management,
//! and subprocess plumbing shared by the `moat docker` subcommands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::client::ClientError;

pub(super) const COMPOSE_FILE: &str = "docker-compose.yml";
pub(super) const ENV_EXAMPLE_FILE: &str = ".env.example";
pub(super) const NGINX_CONF_FILE: &str = "nginx.conf";

/// Files that must live in the compose work dir for `docker compose up` to
/// succeed. `docker-compose.yml` + `.env.example` are the bootstrap pair;
/// `nginx.conf` is bind-mounted by the frontend service.
pub(super) const INSTALL_ASSETS: &[&str] = &[COMPOSE_FILE, ENV_EXAMPLE_FILE, NGINX_CONF_FILE];
pub(super) const ENV_FILE: &str = ".env";
pub(super) const REPO_COMPOSE_SUBDIR: &str = "docker-compose";
pub(super) const DEFAULT_PROJECT_NAME: &str = "moatless";

/// How the compose work-dir was resolved. Reported back in JSON so the skill
/// can tell the user "we found your repo checkout" vs "we're using the managed
/// cache dir".
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ComposeDirSource {
    /// Explicit `--compose-dir` or `--work-dir` flag.
    Flag,
    /// Found by walking up from cwd to a repo checkout.
    Repo,
    /// Managed cache dir under `$HOME/.moatless/compose/`.
    Cache,
}

impl ComposeDirSource {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            ComposeDirSource::Flag => "flag",
            ComposeDirSource::Repo => "repo",
            ComposeDirSource::Cache => "cache",
        }
    }
}

/// Which compose CLI is on PATH. `docker compose` (v2 plugin) is preferred;
/// fall back to `docker-compose` (v1 standalone) for older installations.
#[derive(Clone, Debug)]
pub(super) enum ComposeCmd {
    V2,
    V1,
}

impl ComposeCmd {
    pub(super) fn program(&self) -> &'static str {
        match self {
            ComposeCmd::V2 => "docker",
            ComposeCmd::V1 => "docker-compose",
        }
    }

    pub(super) fn base_args(&self) -> &'static [&'static str] {
        match self {
            ComposeCmd::V2 => &["compose"],
            ComposeCmd::V1 => &[],
        }
    }

    pub(super) fn label(&self) -> &'static str {
        match self {
            ComposeCmd::V2 => "docker compose",
            ComposeCmd::V1 => "docker-compose",
        }
    }
}

/// Detect which compose CLI is available. `docker compose version` shells out
/// because presence of the `docker` binary alone doesn't guarantee the compose
/// plugin is installed.
pub(super) async fn detect_compose_cmd() -> Result<ComposeCmd, ClientError> {
    if which::which("docker").is_ok() {
        let out = run("docker", &["compose", "version"], None).await;
        if let Ok(o) = out {
            if o.success() {
                return Ok(ComposeCmd::V2);
            }
        }
    }
    if which::which("docker-compose").is_ok() {
        return Ok(ComposeCmd::V1);
    }
    Err(ClientError::Other(
        "Neither `docker compose` (v2 plugin) nor `docker-compose` (v1) found on PATH. \
         Install Docker Engine 24+ with the compose plugin: https://docs.docker.com/compose/install/"
            .to_string(),
    ))
}

/// Ensure a required binary is on PATH, or return a helpful `ClientError`.
pub(super) fn require_binary(name: &'static str) -> Result<(), ClientError> {
    which::which(name).map(|_| ()).map_err(|_| {
        ClientError::Other(format!(
            "`{name}` not found on PATH. Install Docker Engine 24+ and retry: https://docs.docker.com/engine/install/"
        ))
    })
}

/// Resolve the directory that holds `docker-compose.yml` + `.env`.
///
/// Priority (highest wins):
/// 1. Explicit flag (e.g. `--compose-dir /path`)
/// 2. A repo checkout discovered by walking up from cwd
/// 3. `$HOME/.moatless/compose/` — a managed cache dir the CLI may create
pub(super) fn resolve_compose_dir(
    flag: Option<&Path>,
) -> Result<(PathBuf, ComposeDirSource), ClientError> {
    if let Some(p) = flag {
        return Ok((p.to_path_buf(), ComposeDirSource::Flag));
    }

    if let Some(repo_dir) = find_repo_compose_dir() {
        return Ok((repo_dir, ComposeDirSource::Repo));
    }

    let cache = cache_compose_dir()?;
    Ok((cache, ComposeDirSource::Cache))
}

/// Walk up from cwd looking for a repo checkout that contains
/// `docker-compose/docker-compose.yml`.
fn find_repo_compose_dir() -> Option<PathBuf> {
    let mut cur = std::env::current_dir().ok()?;
    loop {
        let candidate = cur.join(REPO_COMPOSE_SUBDIR).join(COMPOSE_FILE);
        if candidate.is_file() {
            return Some(cur.join(REPO_COMPOSE_SUBDIR));
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// `$HOME/.moatless/compose/` — the managed cache dir.
pub(super) fn cache_compose_dir() -> Result<PathBuf, ClientError> {
    let home = dirs::home_dir().ok_or_else(|| {
        ClientError::Other("Could not determine home directory (HOME not set?)".to_string())
    })?;
    Ok(home.join(".moatless").join("compose"))
}

/// Copy the two install assets (`docker-compose.yml`, `.env.example`) from the
/// skill's `assets/` dir into a work dir. Symlinks in the skill are followed
/// (via `fs::copy`), so the work dir always ends up with real file contents.
pub(super) fn copy_assets(
    assets_dir: &Path,
    work_dir: &Path,
    overwrite: bool,
) -> Result<Vec<String>, ClientError> {
    if !assets_dir.is_dir() {
        return Err(ClientError::Other(format!(
            "--assets-from path is not a directory: {}",
            assets_dir.display()
        )));
    }

    fs::create_dir_all(work_dir)
        .map_err(|e| ClientError::Other(format!("Failed to create {}: {e}", work_dir.display())))?;

    let mut copied = Vec::new();
    for name in INSTALL_ASSETS.iter().copied() {
        let src = assets_dir.join(name);
        let dst = work_dir.join(name);

        if !src.exists() {
            return Err(ClientError::Other(format!(
                "Required asset missing from skill: {}",
                src.display()
            )));
        }

        if dst.exists() && !overwrite {
            continue;
        }

        // Guard against the skill's assets being symlinks back to the
        // destination (repo-checkout install where assets_dir lives inside the
        // repo). fs::copy would truncate the destination before reading, so
        // source and dest resolving to the same inode clobbers the file.
        if let (Ok(src_canon), Ok(dst_canon)) = (fs::canonicalize(&src), fs::canonicalize(&dst)) {
            if src_canon == dst_canon {
                continue;
            }
        }

        fs::copy(&src, &dst).map_err(|e| {
            ClientError::Other(format!(
                "Failed to copy {} → {}: {e}",
                src.display(),
                dst.display()
            ))
        })?;
        copied.push(name.to_string());
    }

    Ok(copied)
}

/// Output of a completed subprocess run.
pub(super) struct CmdOutput {
    pub(super) status: i32,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl CmdOutput {
    pub(super) fn success(&self) -> bool {
        self.status == 0
    }
}

/// Run a `docker compose ...` command against the given work dir. Always
/// passes `-f <dir>/docker-compose.yml --env-file <dir>/.env -p <project>` so
/// the command works regardless of cwd.
pub(super) async fn run_compose(
    compose: &ComposeCmd,
    work_dir: &Path,
    extra_args: &[&str],
) -> Result<CmdOutput, ClientError> {
    let compose_path = work_dir.join(COMPOSE_FILE);
    let env_path = work_dir.join(ENV_FILE);
    let compose_path_str = compose_path.to_string_lossy().into_owned();
    let env_path_str = env_path.to_string_lossy().into_owned();

    let mut args: Vec<&str> = Vec::new();
    args.extend_from_slice(compose.base_args());
    args.extend_from_slice(&["-f", &compose_path_str]);
    if env_path.is_file() {
        args.extend_from_slice(&["--env-file", &env_path_str]);
    }
    args.extend_from_slice(&["-p", DEFAULT_PROJECT_NAME]);
    args.extend_from_slice(extra_args);

    run(compose.program(), &args, None).await
}

/// Stream a compose subcommand to inherited stdout/stderr. Used for `logs -f`
/// and long-running `up` where the user benefits from live output.
pub(super) async fn run_compose_streaming(
    compose: &ComposeCmd,
    work_dir: &Path,
    extra_args: &[&str],
) -> Result<(), ClientError> {
    let compose_path = work_dir.join(COMPOSE_FILE);
    let env_path = work_dir.join(ENV_FILE);
    let compose_path_str = compose_path.to_string_lossy().into_owned();
    let env_path_str = env_path.to_string_lossy().into_owned();

    let mut args: Vec<&str> = Vec::new();
    args.extend_from_slice(compose.base_args());
    args.extend_from_slice(&["-f", &compose_path_str]);
    if env_path.is_file() {
        args.extend_from_slice(&["--env-file", &env_path_str]);
    }
    args.extend_from_slice(&["-p", DEFAULT_PROJECT_NAME]);
    args.extend_from_slice(extra_args);

    let program = compose.program();
    let label = compose.label();

    let mut child = Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| ClientError::Other(format!("Failed to spawn {label}: {e}")))?;

    let status = child
        .wait()
        .await
        .map_err(|e| ClientError::Other(format!("Failed to await {label}: {e}")))?;

    if !status.success() {
        return Err(ClientError::Other(format!(
            "{label} exited with status {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

/// Run `docker <args>` directly (not compose). Used by `status` for `pg_isready`
/// and `redis-cli PING` invocations on running containers.
pub(super) async fn run_docker(args: &[&str]) -> Result<CmdOutput, ClientError> {
    run("docker", args, None).await
}

async fn run(cmd: &str, args: &[&str], stdin: Option<&str>) -> Result<CmdOutput, ClientError> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }

    let mut child = command
        .spawn()
        .map_err(|e| ClientError::Other(format!("Failed to spawn {cmd}: {e}")))?;

    if let Some(input) = stdin {
        if let Some(mut h) = child.stdin.take() {
            h.write_all(input.as_bytes())
                .await
                .map_err(|e| ClientError::Other(format!("Failed to write stdin to {cmd}: {e}")))?;
            h.shutdown()
                .await
                .map_err(|e| ClientError::Other(format!("Failed to close stdin for {cmd}: {e}")))?;
        }
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| ClientError::Other(format!("Failed to collect {cmd} output: {e}")))?;

    Ok(CmdOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Check whether a TCP port on 127.0.0.1 is already in use. Used by preflight
/// to warn about conflicts before `docker compose up` fails with an opaque
/// bind error.
pub(super) async fn port_in_use(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_dir_source_roundtrip() {
        for s in [
            ComposeDirSource::Flag,
            ComposeDirSource::Repo,
            ComposeDirSource::Cache,
        ] {
            let j = serde_json::to_string(&s).unwrap();
            let back: ComposeDirSource = serde_json::from_str(&j).unwrap();
            assert_eq!(s.as_str(), back.as_str());
        }
    }

    #[test]
    fn copy_assets_writes_files() {
        let tmp = tempfile::tempdir().unwrap();
        let assets = tmp.path().join("assets");
        let work = tmp.path().join("work");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join(COMPOSE_FILE), "services: {}\n").unwrap();
        fs::write(assets.join(ENV_EXAMPLE_FILE), "FOO=bar\n").unwrap();

        let copied = copy_assets(&assets, &work, false).unwrap();
        assert_eq!(copied.len(), 2);
        assert!(work.join(COMPOSE_FILE).is_file());
        assert!(work.join(ENV_EXAMPLE_FILE).is_file());

        let second = copy_assets(&assets, &work, false).unwrap();
        assert!(second.is_empty(), "should skip existing files without overwrite");
    }
}

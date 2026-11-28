//! Docker and `docker compose` subprocess plumbing for the `moat docker`
//! subcommands.
//!
//! Centralises binary detection, compose-dir resolution, `.env` management, and
//! subprocess invocation shared across those subcommands.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::client::ClientError;
use crate::commands::subprocess::{CmdOutput, run, run_streaming};

pub(super) const COMPOSE_FILE: &str = "docker-compose.yml";
pub(super) const ENV_EXAMPLE_FILE: &str = ".env.example";

/// Embedded templates for the Compose work directory.
pub(super) const INSTALL_ASSETS: &[(&str, &str)] = &[
    (
        COMPOSE_FILE,
        include_str!("../../../assets/docker-compose.yml"),
    ),
    (
        ENV_EXAMPLE_FILE,
        include_str!("../../../assets/compose.env.example"),
    ),
];
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
/// Priority: an explicit flag, then a repo checkout found by walking up from
/// cwd, then `$HOME/.moatless/compose/` (a managed cache dir the CLI may
/// create).
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

/// Seed the compose work dir with the install assets.
///
/// Defaults to the compiled-in [`INSTALL_ASSETS`]; pass `custom_from` to read
/// from a directory on disk instead (symlinks are followed via `fs::copy`).
/// Existing files are left untouched unless `overwrite` is set.
pub(super) fn seed_assets(
    work_dir: &Path,
    custom_from: Option<&Path>,
    overwrite: bool,
) -> Result<Vec<String>, ClientError> {
    if let Some(dir) = custom_from {
        if !dir.is_dir() {
            return Err(ClientError::Other(format!(
                "--assets-from path is not a directory: {}",
                dir.display()
            )));
        }
    }

    fs::create_dir_all(work_dir)
        .map_err(|e| ClientError::Other(format!("Failed to create {}: {e}", work_dir.display())))?;

    let mut copied = Vec::new();
    for (name, embedded) in INSTALL_ASSETS.iter().copied() {
        let dst = work_dir.join(name);

        if dst.exists() && !overwrite {
            continue;
        }

        match custom_from {
            Some(dir) => {
                let src = dir.join(name);
                if !src.exists() {
                    return Err(ClientError::Other(format!(
                        "Required asset missing from --assets-from bundle: {}",
                        src.display()
                    )));
                }

                // Guard against the bundle's file being a symlink back to the
                // destination (an --assets-from dir that lives inside the same
                // repo checkout). fs::copy truncates the destination before
                // reading, so source and dest resolving to the same inode would
                // clobber the file.
                if let (Ok(src_canon), Ok(dst_canon)) =
                    (fs::canonicalize(&src), fs::canonicalize(&dst))
                {
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
            }
            None => {
                fs::write(&dst, embedded).map_err(|e| {
                    ClientError::Other(format!("Failed to write {}: {e}", dst.display()))
                })?;
            }
        }
        copied.push(name.to_string());
    }

    Ok(copied)
}

/// Run a `docker compose ...` command against the given work dir. Always passes
/// `-f <dir>/docker-compose.yml --env-file <dir>/.env -p <project>` so the
/// command works regardless of cwd.
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

    run_streaming(compose.program(), compose.label(), &args).await
}

/// Run `docker <args>` directly (not compose). Used by `status` for the
/// `pg_isready` invocation on a running container.
pub(super) async fn run_docker(args: &[&str]) -> Result<CmdOutput, ClientError> {
    run("docker", args, None).await
}

/// Check whether a TCP port on 127.0.0.1 is already in use. Used by preflight
/// to warn about conflicts before `docker compose up` fails with an opaque bind
/// error.
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
    fn seed_assets_writes_every_install_asset() {
        // Seeding with no --assets-from override must materialise every entry
        // in INSTALL_ASSETS. Not a tautology: INSTALL_ASSETS's `include_str!`
        // anchors each entry to a real file, so this fails to *build* if one
        // loses its backing file, and fails at *runtime* if seeding stops
        // writing one or writes the wrong bytes.
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");

        let copied = seed_assets(&work, None, false).unwrap();
        assert_eq!(copied.len(), INSTALL_ASSETS.len());
        for (name, contents) in INSTALL_ASSETS.iter().copied() {
            let dst = work.join(name);
            assert!(dst.is_file(), "{name} not written");
            assert_eq!(
                fs::read_to_string(&dst).unwrap(),
                contents,
                "{name} content does not match the embedded copy"
            );
        }

        let second = seed_assets(&work, None, false).unwrap();
        assert!(
            second.is_empty(),
            "should skip existing files without overwrite"
        );
    }

    #[test]
    fn seed_assets_override_reads_from_disk() {
        // --assets-from overrides the embedded copies with files from a bundle
        // dir; the work dir must end up with the bundle's bytes, not the
        // embedded ones.
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("bundle");
        let work = tmp.path().join("work");
        fs::create_dir_all(&bundle).unwrap();
        for (name, _) in INSTALL_ASSETS.iter().copied() {
            fs::write(bundle.join(name), format!("custom {name}\n")).unwrap();
        }

        let copied = seed_assets(&work, Some(&bundle), false).unwrap();
        assert_eq!(copied.len(), INSTALL_ASSETS.len());
        for (name, _) in INSTALL_ASSETS.iter().copied() {
            assert_eq!(
                fs::read_to_string(work.join(name)).unwrap(),
                format!("custom {name}\n"),
                "{name} should carry the bundle's content"
            );
        }
    }

    #[test]
    fn seed_assets_rejects_non_directory_override() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        let missing = tmp.path().join("nope");
        let err = seed_assets(&work, Some(&missing), false).unwrap_err();
        assert!(
            err.to_string()
                .contains("--assets-from path is not a directory"),
            "unexpected error: {err}"
        );
    }
}

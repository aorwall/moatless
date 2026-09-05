//! The start script every preview-server sidecar runs.
//!
//! Shared by the K8s pod builder and the Docker container builder so both
//! runtimes get identical restart semantics.
//!
//! The script is a supervisor rather than the bare start command, because the
//! start command and env are otherwise baked into the container spec: changing
//! either would mean rebuilding the pod, which restarts the agent container
//! too. Instead the sidecar re-reads an override from the workspace volume
//! whenever the sandbox bumps a generation counter, and re-execs its child in
//! place.
//!
//! The load-bearing invariant is that **only a generation bump restarts the
//! child**. A server that dies on its own still takes the container down with
//! it, so readiness-timeout detection and crash-loop auto-disable
//! (`k8s/pod_watcher.rs`) keep working, and a user-initiated restart never
//! consumes the crash budget.
//!
//! It is bash rather than a helper binary because `docker_image` is per-server:
//! a sidecar may run an image that has no moatless binaries in it.

/// Where the sandbox writes the shared env file. An `emptyDir` in K8s, so
/// nothing here survives the pod — fine for secrets the sandbox rewrites on
/// every start, wrong for anything the user set.
const DATA_DIR: &str = "/opt/moatless/data";

/// Where the sandbox writes per-server overrides, mirroring
/// `crates/sandbox/src/server_control.rs`. On the workspace volume rather than
/// `DATA_DIR` so an override outlives the pod: a stop/start, a redeploy, and the
/// reaper's idle-stop all delete it. Dot-prefixed at the workspace root, a
/// sibling of the repo checkouts rather than inside one.
///
/// Hardcoded for the same reason `DATA_DIR` is: a sidecar may run any
/// `docker_image`, so the script cannot depend on the sandbox image's env. Both
/// builders mount the workspace here.
const CONTROL_DIR: &str = "/opt/moatless/workspace/.moatless/servers";

/// Build the `bash -c` script for one server's sidecar.
///
/// `start_commands` are joined with `&&` and become the fallback used whenever
/// no override is present.
pub fn build_server_start_script(server_name: &str, start_commands: &[String]) -> String {
    let default_cmd = shell_single_quote(&start_commands.join(" && "));
    let ctl = shell_single_quote(&format!("{CONTROL_DIR}/{server_name}"));

    // Notes on the shell below:
    // - `set -m` puts each child in its own process group, so the TERM on
    //   restart reaches the whole tree (e.g. bun -> vite) rather than just the
    //   wrapper's direct child.
    // - The subshell re-sources both env files on every start, so `server.env`
    //   (secrets, refreshed every 15 min) and the override are always current.
    // - The override is sourced last so it wins over both the container env and
    //   `server.env`, and so `unset` lines can drop container-injected vars.
    // - If the child exits at the same instant the generation is bumped we take
    //   the exit path and the container restarts, which applies the new config
    //   anyway — the race is benign in both directions.
    format!(
        r#"set -m
CTL={ctl}
DEFAULT_CMD={default_cmd}
until [ -f {DATA_DIR}/.workspace-ready ]; do sleep 2; done
while :; do
  gen=$(cat "$CTL/generation" 2>/dev/null || echo 0)
  cmd=$(cat "$CTL/command" 2>/dev/null)
  [ -n "$cmd" ] || cmd=$DEFAULT_CMD
  ( if [ -f {DATA_DIR}/server.env ]; then set -a; . {DATA_DIR}/server.env; set +a; fi
    if [ -f "$CTL/env" ]; then set -a; . "$CTL/env"; set +a; fi
    cd "${{WORKSPACE_PATH:-${{WORKSPACE_DIR:-/opt/moatless/workspace}}}}" || exit 1
    exec bash -c "$cmd" ) &
  child=$!
  while kill -0 "$child" 2>/dev/null; do
    [ "$(cat "$CTL/generation" 2>/dev/null || echo 0)" = "$gen" ] || break
    sleep 1
  done
  if kill -0 "$child" 2>/dev/null; then
    echo "[moatless] server config changed - restarting in place"
    kill -TERM -"$child" 2>/dev/null
    ( sleep 10; kill -KILL -"$child" 2>/dev/null ) >/dev/null 2>&1 &
    wait "$child" 2>/dev/null
    continue
  fi
  wait "$child"; code=$?
  echo "[moatless] server exited on its own (code=$code)"
  exit "$code"
done"#
    )
}

/// Wrap a value in single quotes for safe embedding in the script.
fn shell_single_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(cmds: &[&str]) -> String {
        build_server_start_script(
            "dev",
            &cmds.iter().map(|c| (*c).to_string()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn waits_for_the_workspace_ready_sentinel_before_starting() {
        let s = script(&["bun run dev"]);
        assert!(s.contains("until [ -f /opt/moatless/data/.workspace-ready ]; do sleep 2; done"));
        // The wait must precede the supervisor loop, or the first start races
        // setup commands.
        assert!(s.find(".workspace-ready").unwrap() < s.find("while :;").unwrap());
    }

    #[test]
    fn sources_the_override_after_server_env_so_it_wins() {
        let s = script(&["bun run dev"]);
        let server_env = s.find("/opt/moatless/data/server.env; set +a").unwrap();
        let override_env = s.find(r#". "$CTL/env"; set +a"#).unwrap();
        assert!(server_env < override_env);
    }

    #[test]
    fn workspace_path_fallback_points_at_workspace_root() {
        let s = script(&["bun run dev"]);
        assert!(s.contains(r#"cd "${WORKSPACE_PATH:-${WORKSPACE_DIR:-/opt/moatless/workspace}}""#));
    }

    #[test]
    fn joins_multiple_start_commands_with_and() {
        let s = script(&["bun install", "bun run dev"]);
        assert!(s.contains("DEFAULT_CMD='bun install && bun run dev'"));
    }

    #[test]
    fn override_command_takes_precedence_over_the_baked_default() {
        let s = script(&["bun run dev"]);
        assert!(s.contains(r#"cmd=$(cat "$CTL/command" 2>/dev/null)"#));
        assert!(s.contains(r#"[ -n "$cmd" ] || cmd=$DEFAULT_CMD"#));
    }

    #[test]
    fn escapes_single_quotes_in_start_commands() {
        let s = script(&["echo 'hi'"]);
        assert!(s.contains(r#"DEFAULT_CMD='echo '\''hi'\'''"#));
    }

    /// The path is a contract with `crates/sandbox/src/server_control.rs`, which
    /// writes these files. It lives on the workspace volume, not the
    /// `emptyDir` data volume, so an override outlives the pod.
    #[test]
    fn control_path_is_namespaced_per_server_under_the_workspace() {
        let s = build_server_start_script("api", &["serve".to_string()]);
        assert!(s.contains("CTL='/opt/moatless/workspace/.moatless/servers/api'"));
        assert!(!s.contains("/opt/moatless/data/servers"));
    }

    #[test]
    fn a_child_that_exits_on_its_own_takes_the_container_down() {
        // Crash-loop detection and readiness timeouts both key off the
        // container exiting, so the supervisor must not swallow a crash.
        let s = script(&["bun run dev"]);
        assert!(s.contains(r#"wait "$child"; code=$?"#));
        assert!(s.contains(r#"exit "$code""#));
    }

    #[test]
    fn restart_signals_the_whole_process_group() {
        let s = script(&["bun run dev"]);
        assert!(s.starts_with("set -m\n"), "job control must be on");
        assert!(s.contains(r#"kill -TERM -"$child""#));
        assert!(s.contains(r#"kill -KILL -"$child""#));
    }
}

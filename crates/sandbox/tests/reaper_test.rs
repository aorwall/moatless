//! [`moatless_sandbox::reaper::run_as_init`] as PID 1 runs it, from a child
//! subreaper, which the kernel hands orphans to the same way.
//!
//! `harness = false`: libtest's threads would leave `SIGCHLD` unblocked
//! somewhere in the process, and the reaper would never see it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

#[cfg(target_os = "linux")]
fn main() {
    use moatless_sandbox::reaper::run_as_init;
    use std::process::Command;

    nix::sys::prctl::set_child_subreaper(true).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("grandchild.pid");

    // The middle shell exits at once, so `sleep` is orphaned onto this
    // process. Once killed it must leave `/proc` within 1 s: a zombie nobody
    // reaps keeps its entry until its parent waits on it.
    let orphan = r#"
sh -c 'sleep 30 & echo $! > "$1"' _ "$1"
g=$(cat "$1")
kill "$g"
i=0
while [ -e "/proc/$g" ]; do
  i=$((i + 1)); [ "$i" -ge 20 ] && exit 1
  sleep 0.05
done
exit 3"#;
    let code = run_as_init(Command::new("sh").args(["-c", orphan, "_"]).arg(&pidfile)).unwrap();
    assert_eq!(code, 3, "the killed grandchild stayed <defunct> (exit 1)");
    println!("ok    killed orphan reaped within 1 s; child exit code 3 passed through");

    let killed = run_as_init(Command::new("sh").args(["-c", "kill -KILL $$"])).unwrap();
    assert_eq!(killed, 128 + 9);
    println!("ok    a child killed by SIGKILL reports 137");

    // `SIGTERM` sent to the reaper reaches the child, whose trap exits 5.
    let forwarded = r#"trap 'exit 5' TERM; kill -TERM "$PPID"; while :; do sleep 0.05; done"#;
    let code = run_as_init(Command::new("sh").args(["-c", forwarded])).unwrap();
    assert_eq!(code, 5);
    println!("ok    SIGTERM forwarded to the child");
}

#[cfg(not(target_os = "linux"))]
fn main() {}

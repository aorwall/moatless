//! PID 1 duty for a container that runs this binary as its command.
//!
//! The kernel reparents every orphan in the container to PID 1, and only PID 1
//! can reap it. Rather than reaping inside the sandbox, where a `waitpid(-1)`
//! would steal the exit status of a `moat cmd` child Tokio is awaiting, PID 1
//! re-runs this binary as its one child and does nothing but reap and forward
//! `SIGTERM`/`SIGINT`, as `tini` would.

use nix::sys::signal::{SigSet, Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use std::process::Command;

/// Only the signals the sandbox handles (`crate::shutdown`). Any other signal
/// stays blocked here: forwarded, its default action would kill the sandbox,
/// where PID 1 used to ignore it.
const FORWARDED: [Signal; 2] = [Signal::SIGTERM, Signal::SIGINT];

/// As PID 1, runs the sandbox as a child through [`run_as_init`] and returns
/// its exit status; otherwise, or when the child cannot start, `None`.
///
/// Call it first in `main`, before any thread exists or the environment is
/// changed: the child must start from the environment the container gave.
pub fn supervise_if_pid1() -> Option<i32> {
    if std::process::id() != 1 {
        return None;
    }
    let mut sandbox = Command::new(std::env::current_exe().ok()?);
    sandbox.args(std::env::args_os().skip(1));
    run_as_init(&mut sandbox).ok()
}

/// Spawns `command`, reaps every process reparented here until it exits, and
/// returns its status the way a shell reports it: the code, or 128 + signal.
///
/// The calling process must be single-threaded: a `SIGCHLD` sent to a thread
/// that has not blocked it is discarded, and the wait never returns. Signals
/// are unblocked again when the spawn fails.
pub fn run_as_init(command: &mut Command) -> std::io::Result<i32> {
    let mut signals = SigSet::empty();
    for signal in FORWARDED {
        signals.add(signal);
    }
    signals.add(Signal::SIGCHLD);
    // Blocked before the spawn, so a child that exits at once still leaves a
    // pending `SIGCHLD`. `Command::spawn` clears the mask in the child.
    signals.thread_block()?;
    let child = command
        .spawn()
        .and_then(|child| i32::try_from(child.id()).map_err(std::io::Error::other));
    match child {
        Ok(pid) => Ok(supervise(&signals, Pid::from_raw(pid))),
        Err(error) => {
            let _ = signals.thread_unblock();
            Err(error)
        }
    }
}

/// Waits on `signals`, blocked by the caller, until `child` exits.
fn supervise(signals: &SigSet, child: Pid) -> i32 {
    loop {
        match signals.wait() {
            Ok(Signal::SIGCHLD) => {
                if let Some(code) = reap(child) {
                    return code;
                }
            }
            Ok(signal) => {
                let _ = kill(child, signal);
            }
            Err(_) => {}
        }
    }
}

/// Reaps every exited process, since one pending `SIGCHLD` can stand for many,
/// and returns `child`'s status if it was among them.
fn reap(child: Pid) -> Option<i32> {
    let mut status = None;
    loop {
        match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, code)) if pid == child => status = Some(code),
            Ok(WaitStatus::Signaled(pid, signal, _)) if pid == child => {
                status = Some(128 + signal as i32);
            }
            // `StillAlive` once nothing is left to reap, `ECHILD` once no
            // child is left at all.
            Ok(WaitStatus::StillAlive) | Err(_) => return status,
            Ok(_) => {}
        }
    }
}

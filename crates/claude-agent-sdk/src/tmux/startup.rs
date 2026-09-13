use super::session::TmuxSession;
use super::tailer::HookState;
use super::trust::{scan_for_bypass_dialog, scan_for_trust_dialog};
use super::writer::TmuxPasteWriter;
use crate::error::{Result, SdkError};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

const TRUST_POLL_INTERVAL: Duration = Duration::from_millis(250);

pub(super) async fn await_readiness(
    session: &Arc<TmuxSession>,
    writer: &Arc<TmuxPasteWriter>,
    state_rx: watch::Receiver<HookState>,
    timeout: Duration,
) -> Result<()> {
    let trust_token = CancellationToken::new();
    let (pane_tx, pane_rx) = watch::channel(Vec::new());
    let trust_handle = tokio::spawn(startup_dialog_watcher(
        Arc::clone(session),
        Arc::clone(writer),
        trust_token.clone(),
        pane_tx,
    ));
    let result = wait_for_session_start(state_rx, session, timeout).await;
    trust_token.cancel();
    let _ = trust_handle.await;
    if let Err(error) = result {
        let pane = capture_startup_pane(session, &pane_rx).await;
        log_startup_failure(session, &pane, &error);
        session.kill().await;
        return Err(error.with_pane_tail(pane));
    }
    Ok(())
}

/// Upper bound on pane output included in startup-failure logs.
const PANE_CAPTURE_MAX_BYTES: usize = 2048;

/// Tries a live capture first (for a still-alive
/// [`SdkError::SessionStartTimeout`]), falling back to
/// [`startup_dialog_watcher`]'s last snapshot when the session already died;
/// returns `None` when only whitespace was captured.
async fn capture_startup_pane(
    session: &TmuxSession,
    last_pane: &watch::Receiver<Vec<u8>>,
) -> Option<String> {
    let live = session
        .capture_pane_stripped()
        .await
        .filter(|b| !b.iter().all(u8::is_ascii_whitespace));
    let bytes = match live {
        Some(bytes) => bytes,
        None => last_pane.borrow().clone(),
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    Some(pane_tail_lossy(&bytes, PANE_CAPTURE_MAX_BYTES))
}

/// Log the captured tmux pane contents when the session fails to reach
/// `SessionStart`, so the exit reason isn't lost with the pane.
///
/// The same `pane` is attached to the returned error, so it also survives in
/// the task's error event.
fn log_startup_failure(session: &TmuxSession, pane: &Option<String>, error: &SdkError) {
    if let Some(tail) = pane {
        warn!(
            session_name = %session.session_name,
            error = %error,
            pane_output = %tail,
            "tmux session failed to start — captured pane output"
        );
    } else {
        warn!(
            session_name = %session.session_name,
            error = %error,
            "tmux session failed to start; no pane output captured"
        );
    }
}

/// Render the tail of captured pane bytes as lossy UTF-8, capped at `max_bytes`
/// (the most recent output is the most useful part).
fn pane_tail_lossy(bytes: &[u8], max_bytes: usize) -> String {
    let start = bytes.len().saturating_sub(max_bytes);
    let text = String::from_utf8_lossy(&bytes[start..]);
    let trimmed = text.trim();
    if start > 0 {
        format!("…{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Interval at which [`wait_for_session_start`] polls the tmux session's
/// liveness while waiting for the `SessionStart` hook.
const LIVENESS_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Liveness is polled every [`LIVENESS_POLL_INTERVAL`]; if the session dies
/// first (e.g. `claude --continue` finding nothing to continue), returns
/// [`SdkError::TmuxSessionDied`] within one poll interval instead of blocking
/// for the full timeout, so callers can retry quickly.
async fn wait_for_session_start(
    mut state_rx: watch::Receiver<HookState>,
    session: &TmuxSession,
    timeout: Duration,
) -> Result<()> {
    // The pane is captured by the caller's failure handler (which has the
    // watcher's last snapshot); these constructors leave it empty.
    let timeout_err = || SdkError::SessionStartTimeout {
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        pane_tail: None,
    };
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if matches!(*state_rx.borrow(), HookState::SessionStarted { .. }) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(timeout_err());
        }
        // Wake on a hook-state change, but no later than the next liveness poll
        // (and never past the overall deadline).
        let wait = remaining.min(LIVENESS_POLL_INTERVAL);
        match tokio::time::timeout(wait, state_rx.changed()).await {
            // State changed — re-check at the top of the loop.
            Ok(Ok(())) => continue,
            // Sender dropped — listener exited before SessionStart fired.
            Ok(Err(_)) => return Err(timeout_err()),
            // Poll tick (or final slice) elapsed — check the session is still up.
            Err(_) => {
                if !session.is_alive().await {
                    return Err(SdkError::TmuxSessionDied { pane_tail: None });
                }
            }
        }
    }
}

/// Poll `tmux capture-pane -p` every [`TRUST_POLL_INTERVAL`] for the
/// workspace-trust dialog; send Enter (default "Yes, I trust this folder") and
/// exit when found.
///
/// Cancels itself on `cancellation_token`, which the caller fires right after
/// `SessionStart`, keeping this active only for the brief pre-SessionStart
/// window.
async fn startup_dialog_watcher(
    session: Arc<TmuxSession>,
    writer: Arc<TmuxPasteWriter>,
    cancellation_token: CancellationToken,
    pane_tx: watch::Sender<Vec<u8>>,
) {
    // Two distinct blocking dialogs can appear before SessionStart, in order:
    //   1. workspace-trust ("…trust the files in this folder?") — default is
    //      "Yes", a bare Enter accepts.
    //   2. bypass-permissions warning (from --dangerously-skip-permissions) —
    //      default is "No, exit", so we must move Down to "Yes, I accept" first.
    // Each is dismissed at most once; we keep polling until both are handled or
    // SessionStart cancels us.
    let mut trust_dismissed = false;
    let mut bypass_dismissed = false;
    loop {
        if trust_dismissed && bypass_dismissed {
            return;
        }
        tokio::select! {
            () = cancellation_token.cancelled() => {
                debug!("startup_dialog_watcher: cancelled");
                return;
            }
            () = tokio::time::sleep(TRUST_POLL_INTERVAL) => {}
        }

        // capture_pane_stripped returns None when the session is dying —
        // don't spam logs, just retry on the next tick.
        let Some(stripped) = session.capture_pane_stripped().await else {
            continue;
        };
        // Keep the last non-empty capture around: when the session dies before
        // SessionStart, the pane is unrecoverable afterwards, so this snapshot
        // is the only record of what claude printed before exiting.
        if !stripped.iter().all(u8::is_ascii_whitespace) {
            let _ = pane_tx.send(stripped.clone());
        }

        if !trust_dismissed && scan_for_trust_dialog(&stripped) {
            debug!("startup_dialog_watcher: trust dialog detected — pressing Enter");
            if let Err(e) = writer.send_enter().await {
                warn!(error = %e, "startup_dialog_watcher: failed to send Enter for trust");
            }
            trust_dismissed = true;
            continue;
        }

        if !bypass_dismissed && scan_for_bypass_dialog(&stripped) {
            debug!("startup_dialog_watcher: bypass-permissions dialog detected — selecting accept");
            // Default selection is "No, exit"; move Down to "Yes, I accept".
            if let Err(e) = writer.send_key("Down").await {
                warn!(error = %e, "startup_dialog_watcher: failed to send Down for bypass");
            } else if let Err(e) = writer.send_enter().await {
                warn!(error = %e, "startup_dialog_watcher: failed to send Enter for bypass");
            }
            bypass_dismissed = true;
            continue;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_tail_lossy_short_input_passes_through_trimmed() {
        assert_eq!(pane_tail_lossy(b"  hello world \n", 2048), "hello world");
    }

    #[test]
    fn pane_tail_lossy_truncates_to_tail_with_ellipsis() {
        let bytes = [b"x".repeat(100), b"recent output".to_vec()].concat();
        let out = pane_tail_lossy(&bytes, 13);
        assert_eq!(out, "…recent output");
    }

    #[test]
    fn pane_tail_lossy_handles_invalid_utf8() {
        let out = pane_tail_lossy(&[0xff, 0xfe, b'o', b'k'], 2048);
        assert!(
            out.ends_with("ok"),
            "lossy conversion should keep ascii tail: {out}"
        );
    }
}

//! `StdinWriter` implementation that delivers user text to the TUI via the
//! tmux paste buffer.
//!
//! For each user message we:
//! 1. `tmux load-buffer -b <name> -` (text streamed over stdin).
//! 2. `tmux paste-buffer -t <session>:0 -b <name> -p -d` (bracket-paste so
//!    embedded newlines aren't treated as submit; `-d` deletes the buffer
//!    after).
//! 3. `tmux send-keys -t <session>:0 Enter` to submit the prompt.
//!
//! Control envelopes from `QueryHandle` (interrupt, `set_permission_mode`, etc.)
//! are not supported by the TUI; we log and drop them.

use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};

use super::session::tmux_command;
use crate::error::{Result, SdkError};
use crate::transport::StdinWriter;

pub(crate) struct TmuxPasteWriter {
    pub(crate) session_name: String,
    /// The session's private tmux server socket — must match the one
    /// [`super::session::TmuxSession`] was spawned on so paste/send-keys reach it.
    pub(crate) socket: PathBuf,
}

impl TmuxPasteWriter {
    /// Send a single named key (tmux key syntax, e.g. `Enter`, `Down`) to the
    /// pane. Used to drive startup dialogs (trust / bypass-permissions) and to
    /// submit prompts.
    pub(crate) async fn send_key(&self, key: &str) -> Result<()> {
        let status = tmux_command(&self.socket)
            .args(["send-keys", "-t", self.session_name.as_str(), key])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(SdkError::SpawnError)?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux send-keys {key} exited with status {status:?}"
            )));
        }
        Ok(())
    }

    /// Send a bare Enter keypress to the tmux pane. Used to dismiss the
    /// workspace-trust dialog and to submit a previously pasted prompt.
    pub(crate) async fn send_enter(&self) -> Result<()> {
        self.send_key("Enter").await
    }

    /// Paste `text` into the tmux session and press Enter.
    pub(crate) async fn send_text(&self, text: &str) -> Result<()> {
        let buffer_name = format!("sdk-{}", uuid::Uuid::new_v4().simple());

        // 1. Load text into a named tmux buffer via stdin.
        let mut load = tmux_command(&self.socket)
            .args(["load-buffer", "-b", &buffer_name, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    SdkError::TmuxNotFound
                } else {
                    SdkError::SpawnError(e)
                }
            })?;
        if let Some(mut stdin) = load.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(SdkError::SpawnError)?;
            stdin.shutdown().await.map_err(SdkError::SpawnError)?;
        }
        let status = load.wait().await.map_err(SdkError::SpawnError)?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux load-buffer exited with status {status:?}"
            )));
        }

        // Target the session by name; tmux resolves to the active pane in
        // the active window. Do NOT use `:0` — `base-index` may be non-zero.
        let target = self.session_name.as_str();

        // 2. Paste with bracket-paste mode (-p) and delete the buffer after (-d).
        let status = tmux_command(&self.socket)
            .args(["paste-buffer", "-t", target, "-b", &buffer_name, "-p", "-d"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(SdkError::SpawnError)?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux paste-buffer exited with status {status:?}"
            )));
        }

        // 3. Submit. We deliberately do NOT press Enter on a fixed delay here:
        // Ink applies a bracketed-paste / burst-input heuristic, so an Enter
        // that arrives before the paste has rendered into the input box is
        // absorbed as input instead of submitting — leaving the prompt unsent
        // (claude never runs a turn, no transcript, no Stop hook → JSONL
        // discovery times out with JsonlNotFound). Instead, wait for the pasted
        // text to actually appear in the input box, THEN press Enter and confirm
        // the box clears.
        self.submit_pasted_text(target, text).await;

        debug!(target = %target, bytes = text.len(), "Pasted user message into tmux");
        Ok(())
    }

    /// Submit a freshly-pasted prompt: wait for the paste to render into the
    /// input box, then press Enter and confirm the box clears.
    ///
    /// Two phases:
    /// 1. **Land** — poll the pane until the pasted text is visible in the input
    ///    region. Pressing Enter only after the paste has rendered keeps the
    ///    keypress out of Ink's bracketed-paste burst (where it would be
    ///    absorbed) and makes the post-submit "input cleared" check unambiguous
    ///    (empty == submitted, not == still-rendering).
    /// 2. **Submit** — press Enter, then re-send it while the prompt is still
    ///    sitting in the input box (Ink can still occasionally swallow a
    ///    keypress). Re-sending Enter on an already-empty input is a no-op, so
    ///    the budget only costs latency on a genuinely wedged prompt.
    ///
    /// Best-effort: a prompt too short to produce a needle can't be detected in
    /// the pane, so it falls back to a fixed settle + single Enter — behaviour
    /// never regresses below the original single-Enter path.
    async fn submit_pasted_text(&self, target: &str, text: &str) {
        let needle = submit_needle(text);

        // Undetectable (very short) prompt: legacy best-effort path.
        let Some(needle) = needle else {
            tokio::time::sleep(SUBMIT_SETTLE).await;
            if let Err(e) = self.send_enter().await {
                warn!(error = %e, "Enter during submit failed");
            }
            return;
        };

        // Phase 1 — wait for the paste to land in the input box.
        let mut landed = false;
        for _ in 0..SUBMIT_LAND_ATTEMPTS {
            tokio::time::sleep(SUBMIT_LAND_GAP).await;
            if self.prompt_pending(target, Some(&needle)).await {
                landed = true;
                break;
            }
        }
        if !landed {
            // Never positively saw it render. Don't re-paste (risks a doubled
            // message); fall through and try to submit anyway in case it landed
            // just below detection — Phase 2 will catch a late render.
            debug!(
                target = %target,
                "paste not detected in input box within land budget — submitting anyway"
            );
        }

        // Phase 2 — submit and confirm the input box clears.
        for attempt in 0..SUBMIT_CONFIRM_ATTEMPTS {
            if let Err(e) = self.send_enter().await {
                warn!(error = %e, "Enter during submit confirmation failed");
            }
            tokio::time::sleep(SUBMIT_CONFIRM_GAP).await;
            if !self.prompt_pending(target, Some(&needle)).await {
                return; // input region clear — submitted.
            }
            debug!(
                target = %target,
                attempt,
                landed,
                "prompt still in input box — re-sending Enter"
            );
        }

        warn!(
            target = %target,
            landed,
            attempts = SUBMIT_CONFIRM_ATTEMPTS,
            "prompt may not have submitted — input box still non-empty after \
             confirmation budget"
        );
    }

    /// Capture the pane and report whether a prompt is still sitting unsent in
    /// the input region (after the last `❯` marker). Returns true when either
    /// the literal prompt `needle` OR the collapsed-paste placeholder
    /// (`[Pasted text #N +M lines]`, which the TUI shows for multi-line pastes
    /// instead of the literal text) is present. Returns false on any capture
    /// issue, so behaviour never regresses below the single-Enter path.
    async fn prompt_pending(&self, target: &str, needle: Option<&[u8]>) -> bool {
        let output = tmux_command(&self.socket)
            .args(["capture-pane", "-t", target, "-p"])
            .stderr(Stdio::null())
            .output()
            .await;
        let Ok(out) = output else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        let stripped = super::trust::strip_csi(&out.stdout);
        if input_region_contains(&stripped, PASTE_PLACEHOLDER) {
            return true;
        }
        needle.is_some_and(|n| input_region_contains(&stripped, n))
    }
}

/// Poll gap while waiting for a freshly-pasted prompt to render into the input
/// box (Phase 1 of submission).
const SUBMIT_LAND_GAP: std::time::Duration = std::time::Duration::from_millis(100);
/// Max polls waiting for the paste to render before submitting anyway
/// (best-effort). ~4 s budget — paste render is sub-second in practice; this
/// only guards a badly lagging TUI and stays far under the 120 s discovery
/// `init_timeout` that follows.
const SUBMIT_LAND_ATTEMPTS: usize = 40;
/// Fixed settle before the single best-effort Enter, used only for prompts too
/// short to detect in the pane.
const SUBMIT_SETTLE: std::time::Duration = std::time::Duration::from_millis(200);
/// Gap between submit-confirmation retries (Phase 2).
const SUBMIT_CONFIRM_GAP: std::time::Duration = std::time::Duration::from_millis(250);
/// Max number of Enter re-sends before giving up (best-effort). Re-sending Enter
/// on an already-submitted (empty) input is a no-op, so a generous budget only
/// costs latency on a genuinely wedged prompt — ~3 s of retries here.
const SUBMIT_CONFIRM_ATTEMPTS: usize = 12;
/// Max needle length taken from the prompt for input-region matching.
const SUBMIT_NEEDLE_LEN: usize = 32;
/// Minimum needle length; below this the prompt is too short to match reliably.
const SUBMIT_NEEDLE_MIN: usize = 8;
/// The `❯` input-prompt marker (U+276F) preceding the claude TUI readline.
const INPUT_MARKER: &[u8] = "❯".as_bytes();
/// Whitespace-stripped, lowercased form of the TUI's collapsed-paste hint
/// (`[Pasted text #1 +5 lines]`). The TUI renders this instead of the literal
/// text for multi-line pastes, so the literal-prompt needle won't match while
/// the paste sits unsent — this does.
const PASTE_PLACEHOLDER: &[u8] = b"[pastedtext";

/// Build a whitespace-stripped, lowercased needle from the start of the prompt.
/// Whitespace is removed entirely so terminal line-wrapping in the captured
/// pane doesn't break the match. Returns None when too few chars remain.
fn submit_needle(text: &str) -> Option<Vec<u8>> {
    let collapsed: Vec<u8> = text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|b| b.to_ascii_lowercase())
        .take(SUBMIT_NEEDLE_LEN)
        .collect();
    (collapsed.len() >= SUBMIT_NEEDLE_MIN).then_some(collapsed)
}

/// True if `needle` appears in the pane region AFTER the last input marker
/// (`❯`), with whitespace removed. Post-submit the input region is empty and
/// the prompt has scrolled into the history above the marker, so a match means
/// the prompt is still pending submission. `needle` must already be
/// whitespace-stripped and lowercased (see [`submit_needle`]).
fn input_region_contains(stripped: &[u8], needle: &[u8]) -> bool {
    let Some(start) = last_index_of(stripped, INPUT_MARKER) else {
        return false;
    };
    let region: Vec<u8> = stripped[start + INPUT_MARKER.len()..]
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .map(u8::to_ascii_lowercase)
        .collect();
    !needle.is_empty() && region.windows(needle.len()).any(|w| w == needle)
}

/// Index of the last occurrence of `needle` in `haystack`, or None.
fn last_index_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .rev()
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

#[async_trait]
impl StdinWriter for TmuxPasteWriter {
    async fn write_json_line(&self, json: &str) -> Result<()> {
        // The only JSON the SDK writes to the writer in normal flow is a
        // serialized user message (via QueryHandle::send_user_message). Other
        // control envelopes have no equivalent in TUI mode.
        let value: Value = serde_json::from_str(json).map_err(SdkError::JsonError)?;
        let msg_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        if msg_type != "user" {
            warn!(
                msg_type,
                "TmuxPasteWriter dropped non-user message — \
                 control protocol is not supported in tmux mode"
            );
            return Ok(());
        }

        let text = extract_user_text(&value).unwrap_or_default();
        self.send_text(&text).await
    }
}

/// Extract the user-visible text from a serialized `UserMessage` JSON.
/// Handles both string and array-of-blocks `content` shapes.
fn extract_user_text(value: &Value) -> Option<String> {
    let content = value.get("message").and_then(|m| m.get("content"))?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        let joined: String = arr
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    b.get("text").and_then(Value::as_str).map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Some(joined);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_text_from_string_content() {
        let v = json!({
            "type": "user",
            "message": { "role": "user", "content": "hello world" }
        });
        assert_eq!(extract_user_text(&v).as_deref(), Some("hello world"));
    }

    #[test]
    fn extract_text_from_array_content() {
        let v = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "text", "text": "line one"},
                    {"type": "text", "text": "line two"}
                ]
            }
        });
        assert_eq!(extract_user_text(&v).as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn submit_needle_strips_whitespace_and_lowercases() {
        let n = submit_needle("Reply with just OK to confirm").expect("needle");
        // Whitespace removed, lowercased, capped at SUBMIT_NEEDLE_LEN.
        assert_eq!(&n, b"replywithjustoktoconfirm".as_slice());
    }

    #[test]
    fn submit_needle_none_for_short_prompt() {
        assert!(submit_needle("ok").is_none());
        assert!(submit_needle("   \n  ").is_none());
    }

    #[test]
    fn input_region_pending_when_prompt_still_after_marker() {
        // Mirrors the wedged pane captured on the cluster: prompt sits in the
        // input box (after `❯`), unsent.
        let pane = "\
✻ Crunched for 23m 21s
────────────────────────────────
❯ <sent_by>aorwall-frankpenny</sent_by>
  Reply with just OK to confirm the agent is running.
────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle)";
        let needle = submit_needle(
            "<sent_by>aorwall-frankpenny</sent_by>\nReply with just OK to confirm the agent is running.",
        )
        .expect("needle");
        assert!(input_region_contains(pane.as_bytes(), &needle));
    }

    #[test]
    fn input_region_clear_after_submit() {
        // After submit the prompt scrolls into history (above the marker) and
        // the input region (after `❯`) is empty — must NOT report pending.
        let pane = "\
❯ <sent_by>aorwall-frankpenny</sent_by> Reply with just OK to confirm the agent is running.
✻ Working…
────────────────────────────────
❯
────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle)";
        let needle = submit_needle(
            "<sent_by>aorwall-frankpenny</sent_by>\nReply with just OK to confirm the agent is running.",
        )
        .expect("needle");
        assert!(!input_region_contains(pane.as_bytes(), &needle));
    }

    #[test]
    fn input_region_matches_across_line_wrap() {
        // Terminal hard-wraps the input across lines; whitespace removal must
        // rejoin the split tokens so the needle still matches.
        let pane = "\
────────────────────────────────
❯ Reply with just OK to con
  firm the agent is running
────────────────────────────────";
        let needle = submit_needle("Reply with just OK to confirm").expect("needle");
        assert!(input_region_contains(pane.as_bytes(), &needle));
    }

    #[test]
    fn input_region_detects_collapsed_paste_placeholder() {
        // Real pane captured on the cluster (0.1.59): the TUI collapses a
        // multi-line paste into `[Pasted text #1 +5 lines]` instead of showing
        // the literal text, so the prompt needle won't match — but the
        // placeholder check must still flag the input as pending. `0 tokens`
        // confirms no turn ran.
        let pane = "\
 ▝▜█████▛▘  Opus 4.8 · Claude API
────────────────────────────────
❯ [Pasted text #1 +5 lines]
────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle)                0 tokens";
        // Literal needle is absent from the collapsed pane...
        let needle = submit_needle(
            "<sent_by>aorwall-frankpenny</sent_by>\nVERIFY59: reply with exactly the token OK-59.",
        )
        .expect("needle");
        assert!(!input_region_contains(pane.as_bytes(), &needle));
        // ...but the collapsed-paste placeholder is detected.
        assert!(input_region_contains(pane.as_bytes(), PASTE_PLACEHOLDER));
    }

    #[test]
    fn input_region_pending_with_prior_marker_in_history() {
        // Exact shape of a wedge captured on the cluster (0.1.62): a previous
        // turn's prompt (with its own `❯`) sits in the scrollback ABOVE the
        // active input box, which holds the still-unsent prompt. `last_index_of`
        // must lock onto the active (last) `❯`, and the needle must match there
        // despite the box borders + footer that follow it.
        let pane = "\
● STRESS-3-OK
✻ Worked for 1s
❯ <sent_by>aorwall-frankpenny</sent_by>Reply with exactly: STRESS-4-OK
  Reply with exactly: STRESS-5-OK
● STRESS-5-OK
✻ Worked for 1s
────────────────────────────────
❯ <sent_by>aorwall-frankpenny</sent_by>
  Reply with exactly: DIAGN-A-OK
────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle)               60531 tokens";
        // Note: the 32-char needle is dominated by the `<sent_by>…</sent_by>`
        // wrapper (the cluster prepends it to every message), so it matches on
        // the wrapper's presence in the active input region — a reliable
        // "input box non-empty" signal, which is all submit-confirmation needs.
        let needle =
            submit_needle("<sent_by>aorwall-frankpenny</sent_by>\nReply with exactly: DIAGN-A-OK")
                .expect("needle");
        assert!(input_region_contains(pane.as_bytes(), &needle));
    }

    #[test]
    fn input_region_false_without_marker() {
        // No input marker in the capture → can't locate the input region →
        // assume submitted (no spurious re-send).
        let needle = submit_needle("Reply with just OK to confirm").expect("needle");
        assert!(!input_region_contains(
            b"some pane with no marker at all",
            &needle
        ));
    }
}

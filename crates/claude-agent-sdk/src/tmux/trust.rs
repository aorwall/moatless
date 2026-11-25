//! Detect the workspace-trust dialog Claude shows in unfamiliar directories.
//!
//! The dialog blocks startup before `SessionStart` fires, and
//! `--dangerously-skip-permissions` does not suppress it; default selection
//! accepts on a bare Enter.
//!
//! Detection: strip CSI/OSC/DCS escapes (cursor-move padding, not literal
//! spaces) and check for "trust" and "folder". Mirrors `claude-p`'s 1:1.

/// Strip CSI / OSC / DCS escape sequences from `bytes`, leaving just the
/// literal payload for plain-text matching. Not a layout reconstruction — only
/// removes bytes that would otherwise separate adjacent words.
pub(crate) fn strip_csi(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != 0x1b {
            out.push(b);
            i += 1;
            continue;
        }
        if i + 1 >= bytes.len() {
            break;
        }
        let next = bytes[i + 1];
        match next {
            b'[' => {
                // CSI: ESC [ <params> <intermediates> <final>
                i += 2;
                while i < bytes.len() && (0x30..=0x3f).contains(&bytes[i]) {
                    i += 1;
                }
                while i < bytes.len() && (0x20..=0x2f).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // final byte
                }
            }
            b']' => {
                // OSC: ESC ] ... BEL  | ESC ] ... ESC \
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            b'P' | b'X' | b'^' | b'_' => {
                // DCS / SOS / PM / APC: ... ESC \
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            _ => {
                // Two-byte escape (e.g. ESC = for keypad mode) — drop both.
                i += 2;
            }
        }
    }
    out
}

/// Heuristic: both "trust" and "folder" appear post-CSI-stripping — present in
/// the dialog text and absent from the normal REPL prompt, so the combo is
/// specific.
pub(crate) fn scan_for_trust_dialog(stripped: &[u8]) -> bool {
    contains_lower(stripped, b"trust") && contains_lower(stripped, b"folder")
}

/// The default menu selection is "1. No, exit" — a bare Enter would quit — so
/// the watcher must navigate to "2. Yes, I accept". Detection checks for both
/// "bypass" and "accept" post-CSI-stripping; the post-acceptance status line
/// has "bypass" but not "accept", so it won't re-fire once dismissed.
pub(crate) fn scan_for_bypass_dialog(stripped: &[u8]) -> bool {
    contains_lower(stripped, b"bypass") && contains_lower(stripped, b"accept")
}

/// Case-insensitive `needle in haystack`. Both inputs are ASCII; we only
/// lowercase ASCII letters.
fn contains_lower(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return needle.is_empty();
    }
    for window in haystack.windows(needle.len()) {
        if window
            .iter()
            .zip(needle.iter())
            .all(|(h, n)| h.eq_ignore_ascii_case(n))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_csi_passes_through_plain_ascii() {
        assert_eq!(strip_csi(b"hello world"), b"hello world");
    }

    #[test]
    fn strip_csi_removes_simple_csi_sequence() {
        // ESC [ 1 ; 2 m  → SGR (set graphics)
        let input = b"\x1b[1;2mbold\x1b[0m";
        assert_eq!(strip_csi(input), b"bold");
    }

    #[test]
    fn strip_csi_removes_cursor_move_padding() {
        // The trust dialog pads words with `\e[1C` (move cursor forward 1).
        // After stripping the bytes between letters disappear, so adjacent
        // tokens concatenate.
        let input = b"trust\x1b[1Cfolder";
        assert_eq!(strip_csi(input), b"trustfolder");
    }

    #[test]
    fn strip_csi_removes_osc_with_bel_terminator() {
        // ESC ] 0 ; title BEL  (xterm window title)
        let input = b"prefix\x1b]0;some title\x07suffix";
        assert_eq!(strip_csi(input), b"prefixsuffix");
    }

    #[test]
    fn strip_csi_removes_osc_with_st_terminator() {
        // ESC ] 0 ; title ESC \  (ST = String Terminator)
        let input = b"prefix\x1b]0;some title\x1b\\suffix";
        assert_eq!(strip_csi(input), b"prefixsuffix");
    }

    #[test]
    fn strip_csi_removes_dcs() {
        let input = b"a\x1bPdcs payload\x1b\\b";
        assert_eq!(strip_csi(input), b"ab");
    }

    #[test]
    fn strip_csi_handles_trailing_escape_gracefully() {
        // Lone ESC at the end with nothing after — should not panic.
        let input = b"hello\x1b";
        // Just check no panic and either empty or "hello" — we accept either
        // behavior (the byte after ESC is missing).
        let out = strip_csi(input);
        assert!(out.starts_with(b"hello"));
    }

    #[test]
    fn scan_detects_trust_dialog_text() {
        // After stripping, the words "trust" and "folder" appear in the
        // dialog body.
        let stripped = b"Do you trust the files in this folder?";
        assert!(scan_for_trust_dialog(stripped));
    }

    #[test]
    fn scan_detects_after_csi_strip_round_trip() {
        // Simulate a captured pane: CSI padding between every token.
        let raw = b"Do you \x1b[1Ctrust the files in this \x1b[1Cfolder ?";
        let stripped = strip_csi(raw);
        assert!(scan_for_trust_dialog(&stripped));
    }

    #[test]
    fn scan_case_insensitive() {
        let stripped = b"TRUST the files in this FOLDER";
        assert!(scan_for_trust_dialog(stripped));
    }

    #[test]
    fn scan_negative_only_one_token() {
        assert!(!scan_for_trust_dialog(b"the system has trust issues today"));
        assert!(!scan_for_trust_dialog(b"create a folder called bar"));
    }

    #[test]
    fn scan_negative_on_normal_repl_prompt() {
        // A typical empty Claude REPL banner — no "trust" or "folder".
        let normal = "╭───────────────────╮\n│ Welcome to Claude │\n╰───────────────────╯\n> ";
        assert!(!scan_for_trust_dialog(normal.as_bytes()));
    }

    #[test]
    fn scan_detects_bypass_dialog_text() {
        let body = b"WARNING: Claude Code running in Bypass Permissions mode. \
            By proceeding, you accept all responsibility. 1. No, exit  2. Yes, I accept";
        assert!(scan_for_bypass_dialog(body));
    }

    #[test]
    fn scan_detects_bypass_after_csi_strip() {
        let raw = b"Bypass\x1b[1CPermissions mode \x1b[1Cyou \x1b[1Caccept all responsibility";
        let stripped = strip_csi(raw);
        assert!(scan_for_bypass_dialog(&stripped));
    }

    #[test]
    fn scan_bypass_negative_on_status_line() {
        // The post-acceptance status line has "bypass" but not "accept" — must
        // not re-trigger the watcher once the dialog is dismissed.
        let status = b"  bypass permissions on (shift+tab to cycle) - left for agents";
        assert!(!scan_for_bypass_dialog(status));
    }

    #[test]
    fn scan_bypass_negative_on_trust_dialog() {
        let trust = b"Do you trust the files in this folder? 1. Yes, I trust  2. No, exit";
        assert!(!scan_for_bypass_dialog(trust));
    }
}

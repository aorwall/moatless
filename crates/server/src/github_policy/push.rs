//! The ref updates a `git-receive-pack` request opens with, and the
//! report-status that refuses them.
//!
//! The request is pkt-lines up to the first flush (`0000`), then the pack.
//! Only the pkt-lines are buffered; the pack streams on untouched.

use moatless_core::workspace_identity_grants::GitHubCapability;

use super::refusal::{Refusal, capability_rule};
use super::scope::PolicyScope;

/// The most a command list may take before the request is refused unread.
pub(crate) const COMMANDS_LIMIT: usize = 64 * 1024;

/// One `<old> <new> <ref>` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefUpdate {
    pub name: String,
    pub delete: bool,
}

/// A request's command list and the capabilities its first line asks for.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Commands {
    pub updates: Vec<RefUpdate>,
    capabilities: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Parsed {
    /// The command list ended at byte `consumed`.
    Complete {
        commands: Commands,
        consumed: usize,
    },
    NeedMore,
    Invalid(String),
}

fn is_zero_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b == b'0')
}

/// Reads the command list from the start of a receive-pack body.
pub(crate) fn parse(buf: &[u8]) -> Parsed {
    let mut at = 0;
    let mut commands = Commands::default();
    loop {
        let Some(header) = buf.get(at..at + 4) else {
            return Parsed::NeedMore;
        };
        let Some(length) = std::str::from_utf8(header)
            .ok()
            .and_then(|h| usize::from_str_radix(h, 16).ok())
        else {
            return Parsed::Invalid("a pkt-line length is not hex".to_string());
        };
        if length == 0 {
            return Parsed::Complete {
                commands,
                consumed: at + 4,
            };
        }
        if length < 4 {
            return Parsed::Invalid(format!("pkt-line length {length} before the flush"));
        }
        let Some(payload) = buf.get(at + 4..at + length) else {
            return Parsed::NeedMore;
        };
        at += length;
        let Ok(line) = std::str::from_utf8(payload) else {
            return Parsed::Invalid("a command is not UTF-8".to_string());
        };
        let line = line.strip_suffix('\n').unwrap_or(line);
        if line.starts_with("shallow ") {
            continue;
        }
        let (command, capabilities) = line.split_once('\0').unwrap_or((line, ""));
        if commands.updates.is_empty() {
            commands.capabilities = capabilities.split(' ').map(str::to_string).collect();
        }
        let mut parts = command.splitn(3, ' ');
        let (Some(_old), Some(new), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
            return Parsed::Invalid(format!("`{command}` is not `<old> <new> <ref>`"));
        };
        commands.updates.push(RefUpdate {
            name: name.to_string(),
            delete: is_zero_id(new),
        });
    }
}

/// Each update's refusal, if any, for a push to `repository` (`repo_key`).
pub(super) fn check(
    scope: &PolicyScope,
    repository: &str,
    updates: &[RefUpdate],
) -> Vec<(String, Refusal)> {
    let mut refused = Vec::new();
    for update in updates {
        let refusal = if scope.allows(repository) {
            check_ref(scope, repository, update)
        } else {
            Some(Refusal::new(
                "repo_scope",
                "not one of this workspace's repositories; place it, or add it to \
                 githubPolicy.extraRepos in the workspace's grant",
            ))
        };
        if let Some(refusal) = refusal {
            refused.push((update.name.clone(), refusal));
        }
    }
    refused
}

fn check_ref(scope: &PolicyScope, repository: &str, update: &RefUpdate) -> Option<Refusal> {
    let needs = |capability: GitHubCapability| {
        (!scope.has(capability)).then(|| {
            Refusal::new(
                capability_rule(capability),
                format!(
                    "needs the {} capability in githubPolicy.capabilities of the workspace's grant",
                    capability.as_str()
                ),
            )
        })
    };
    if let Some(branch) = update.name.strip_prefix("refs/heads/") {
        if !scope.is_default_branch(repository, branch) {
            return None;
        }
        if update.delete {
            return Some(Refusal::new(
                "push_default_branch_delete",
                "a sandbox never deletes the default branch",
            ));
        }
        return needs(GitHubCapability::PushDefaultBranch);
    }
    if update.name.starts_with("refs/tags/") {
        return needs(GitHubCapability::PushTags);
    }
    Some(Refusal::new(
        "push_ref",
        "a sandbox pushes only branches and tags",
    ))
}

/// Largest side-band payload for each band capability, after the band byte.
const SIDE_BAND_64K: usize = 65_515;
const SIDE_BAND: usize = 995;

fn pkt(payload: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(format!("{:04x}", payload.len() + 4).as_bytes());
    out.extend_from_slice(payload);
}

/// The receive-pack response that rejects every update in `commands`, naming
/// `refused`'s reasons, or `None` when the client asked for no report-status
/// and so could show none.
pub(crate) fn report(commands: &Commands, refused: &[(String, Refusal)]) -> Option<Vec<u8>> {
    let has = |cap: &str| commands.capabilities.iter().any(|c| c == cap);
    if !has("report-status") && !has("report-status-v2") {
        return None;
    }
    let mut status = Vec::new();
    pkt(b"unpack ok\n", &mut status);
    for update in &commands.updates {
        let reason = refused
            .iter()
            .find(|(name, _)| *name == update.name)
            .map_or(
                "another ref in this push was refused".to_string(),
                |(_, r)| r.message.replace(['\n', '\r'], " "),
            );
        pkt(
            format!("ng {} {reason}\n", update.name).as_bytes(),
            &mut status,
        );
    }
    status.extend_from_slice(b"0000");

    let band_limit = if has("side-band-64k") {
        SIDE_BAND_64K
    } else if has("side-band") {
        SIDE_BAND
    } else {
        return Some(status);
    };
    let mut out = Vec::new();
    for chunk in status.chunks(band_limit) {
        let mut payload = Vec::with_capacity(chunk.len() + 1);
        payload.push(1);
        payload.extend_from_slice(chunk);
        pkt(&payload, &mut out);
    }
    out.extend_from_slice(b"0000");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_policy::test_support::scope;

    const OLD: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";
    const ZERO: &str = "0000000000000000000000000000000000000000";
    const REPO: &str = "github.com/soaplabs/moatless";

    fn line(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        pkt(text.as_bytes(), &mut out);
        out
    }

    fn body(lines: &[String]) -> Vec<u8> {
        let mut out: Vec<u8> = lines.iter().flat_map(|l| line(l)).collect();
        out.extend_from_slice(b"0000PACK....");
        out
    }

    fn commands(lines: &[String]) -> Commands {
        match parse(&body(lines)) {
            Parsed::Complete { commands, .. } => commands,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_command_list_ends_at_the_flush_and_the_pack_is_left_alone() {
        let lines = [
            format!("{OLD} {NEW} refs/heads/feature\0report-status side-band-64k agent=git/2\n"),
            format!("{OLD} {ZERO} refs/heads/old\n"),
        ];
        let bytes = body(&lines);
        let Parsed::Complete { commands, consumed } = parse(&bytes) else {
            panic!("incomplete");
        };
        assert_eq!(&bytes[consumed..], b"PACK....");
        assert_eq!(
            commands.updates,
            vec![
                RefUpdate {
                    name: "refs/heads/feature".to_string(),
                    delete: false
                },
                RefUpdate {
                    name: "refs/heads/old".to_string(),
                    delete: true
                },
            ]
        );
        assert_eq!(parse(&bytes[..10]), Parsed::NeedMore);
        assert!(matches!(parse(b"zzzz"), Parsed::Invalid(_)));
    }

    #[test]
    fn a_feature_branch_passes_and_the_default_branch_tags_and_other_refs_need_more() {
        let updates = |name: &str, delete: bool| {
            vec![RefUpdate {
                name: name.to_string(),
                delete,
            }]
        };
        let rules = |caps: &[GitHubCapability], name: &str, delete: bool| {
            check(&scope(caps), REPO, &updates(name, delete))
                .into_iter()
                .map(|(_, r)| r.rule)
                .collect::<Vec<_>>()
        };
        assert!(rules(&[], "refs/heads/feature", false).is_empty());
        assert!(rules(&[], "refs/heads/feature", true).is_empty());
        assert_eq!(
            rules(&[], "refs/heads/develop", false),
            ["push_default_branch"]
        );
        assert_eq!(
            rules(&[], "refs/heads/main", false),
            ["push_default_branch"]
        );
        assert!(
            rules(
                &[GitHubCapability::PushDefaultBranch],
                "refs/heads/main",
                false
            )
            .is_empty()
        );
        assert_eq!(
            rules(
                &[GitHubCapability::PushDefaultBranch],
                "refs/heads/main",
                true
            ),
            ["push_default_branch_delete"]
        );
        assert_eq!(rules(&[], "refs/tags/v1", false), ["push_tags"]);
        assert!(rules(&[GitHubCapability::PushTags], "refs/tags/v1", false).is_empty());
        assert_eq!(rules(&[], "refs/notes/commits", false), ["push_ref"]);
        assert_eq!(
            check(
                &scope(&[]),
                "github.com/soaplabs/other",
                &updates("refs/heads/x", false)
            )[0]
            .1
            .rule,
            "repo_scope"
        );
    }

    #[test]
    fn a_refusal_reports_every_ref_as_rejected_inside_the_side_band() {
        let commands = commands(&[
            format!("{OLD} {NEW} refs/heads/main\0report-status side-band-64k\n"),
            format!("{OLD} {NEW} refs/heads/feature\n"),
        ]);
        let refused = check(&scope(&[]), REPO, &commands.updates);
        let report = report(&commands, &refused).unwrap();
        assert_eq!(report[4], 1, "band 1");
        let text = String::from_utf8_lossy(&report);
        assert!(text.contains("unpack ok\n"));
        assert!(text.contains("ng refs/heads/main needs the pushDefaultBranch capability"));
        assert!(text.contains("ng refs/heads/feature another ref in this push was refused"));
        assert!(report.ends_with(b"00000000"));
    }

    #[test]
    fn without_report_status_there_is_nothing_to_report_into() {
        let commands = commands(&[format!("{OLD} {NEW} refs/heads/main\0side-band-64k\n")]);
        assert_eq!(report(&commands, &[]), None);
        let plain = commands_without_band();
        let report = report(&plain, &[]).unwrap();
        assert!(report.starts_with(b"000eunpack ok\n"));
    }

    fn commands_without_band() -> Commands {
        commands(&[format!("{OLD} {NEW} refs/heads/main\0report-status\n")])
    }
}

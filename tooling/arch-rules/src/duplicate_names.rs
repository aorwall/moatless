//! One public type name must mean one thing.
//!
//! A name exported by two shared-vocabulary crates leaves a reader unable to
//! tell which shape they hold, and the copies drift because nothing fails when
//! one gains a field. Only exported types count — see `public_api`.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::Violation;
use crate::public_api::{Export, exports, root_of};
use crate::workspace::Workspace;

pub const RULE: &str = "duplicate-type-name";

/// The crates whose types are shared vocabulary.
///
/// Two providers may both define `Config` without confusing anyone, because
/// neither name escapes its crate. These seven are named by others, so a
/// collision here is a collision in somebody's import list. Extend this list
/// as a crate becomes shared; do not extend it to silence a violation.
const SHARED: &[&str] = &[
    "moatless-types",
    "moatless-api-schemas",
    "moatless-core",
    "moatless-ports",
    "moatless-adapter-model",
    "moatless-sandbox-runtime",
    "moatless-sandbox",
];

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut per_crate: BTreeMap<String, Vec<Export>> = BTreeMap::new();

    for crate_name in SHARED {
        let Some(member) = ws.member(crate_name) else {
            continue;
        };
        let sources: BTreeMap<String, String> = ws.sources(member)?.into_iter().collect();
        let Some(root) = root_of(&sources, &ws.relative(&member.dir)) else {
            continue;
        };
        per_crate.insert((*crate_name).to_string(), exports(&sources, &root)?);
    }

    Ok(collisions(&per_crate))
}

/// Names exported by more than one of the given crates.
fn collisions(per_crate: &BTreeMap<String, Vec<Export>>) -> Vec<Violation> {
    // name -> crate -> the files defining it
    let mut defined: BTreeMap<&str, BTreeMap<&str, Vec<String>>> = BTreeMap::new();
    for (crate_name, found) in per_crate {
        for export in found {
            defined
                .entry(&export.name)
                .or_default()
                .entry(crate_name)
                .or_default()
                .push(export.file.clone());
        }
    }

    defined
        .into_iter()
        .filter(|(_, crates)| crates.len() > 1)
        .map(|(name, crates)| {
            let locations: Vec<String> = crates.values().flatten().cloned().collect();
            let where_ = crates.keys().copied().collect::<Vec<_>>().join(", ");
            Violation {
                rule: RULE,
                // The crates are part of the identity, not just the message: a
                // key of `Config` alone would let a fourth crate join an
                // already-accepted collision without failing anything. Crate
                // names rather than paths, so moving a file within a crate
                // does not churn the baseline.
                key: format!("{name} in {where_}"),
                message: format!("`{name}` is defined in {} crates: {where_}", crates.len()),
                locations,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crates(entries: &[(&str, &[(&str, &str)])]) -> BTreeMap<String, Vec<Export>> {
        entries
            .iter()
            .map(|(crate_name, found)| {
                let exports = found
                    .iter()
                    .map(|(name, file)| Export {
                        name: (*name).to_string(),
                        file: (*file).to_string(),
                    })
                    .collect();
                ((*crate_name).to_string(), exports)
            })
            .collect()
    }

    #[test]
    fn a_name_in_one_crate_is_not_a_collision() {
        let found = collisions(&crates(&[
            ("moatless-types", &[("Task", "crates/types/src/task.rs")]),
            ("moatless-core", &[("Pool", "crates/core/src/db.rs")]),
        ]));
        assert!(found.is_empty());
    }

    #[test]
    fn a_name_in_two_crates_is() {
        let found = collisions(&crates(&[
            ("moatless-core", &[("Config", "crates/core/src/config.rs")]),
            (
                "moatless-sandbox",
                &[("Config", "crates/sandbox/src/config.rs")],
            ),
        ]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "Config in moatless-core, moatless-sandbox");
        assert_eq!(found[0].locations.len(), 2);
    }

    /// The finding this key shape exists for: a third crate joining an
    /// accepted collision must not land in the accepted slot.
    #[test]
    fn a_new_participant_changes_the_key() {
        let two = crates(&[
            ("moatless-core", &[("Config", "crates/core/src/config.rs")]),
            (
                "moatless-sandbox",
                &[("Config", "crates/sandbox/src/config.rs")],
            ),
        ]);
        let mut three = two.clone();
        three.insert(
            "moatless-types".to_string(),
            vec![Export {
                name: "Config".to_string(),
                file: "crates/types/src/config.rs".to_string(),
            }],
        );

        let before = collisions(&two)[0].id();
        let after = collisions(&three)[0].id();
        assert_ne!(before, after);
    }

    /// A file moving inside a crate must not, or every rename rewrites the
    /// baseline.
    #[test]
    fn a_moved_file_does_not() {
        let before = crates(&[
            ("moatless-core", &[("Config", "crates/core/src/config.rs")]),
            (
                "moatless-sandbox",
                &[("Config", "crates/sandbox/src/config.rs")],
            ),
        ]);
        let after = crates(&[
            (
                "moatless-core",
                &[("Config", "crates/core/src/config/mod.rs")],
            ),
            (
                "moatless-sandbox",
                &[("Config", "crates/sandbox/src/config.rs")],
            ),
        ]);
        assert_eq!(collisions(&before)[0].id(), collisions(&after)[0].id());
    }
}

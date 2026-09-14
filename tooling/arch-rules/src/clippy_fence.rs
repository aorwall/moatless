//! A `disallowed-methods` path into this workspace names an item that exists.
//!
//! Clippy reports nothing for a path it cannot resolve, so renaming the fenced
//! item disarms the fence with nothing red. A path is judged unless its first
//! segment names a crate outside the workspace: an external path has no source
//! here to resolve against and cannot go stale under our own edits — upstream
//! renaming it breaks the call sites instead. A first segment naming neither a
//! member nor a declared dependency is a member we renamed or dropped, which
//! disarms the fence as silently as renaming the method does.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};

use crate::Violation;
use crate::public_api::exports;
use crate::workspace::{Member, Workspace};

pub const RULE: &str = "clippy-fence-resolves";

/// The `clippy.toml` table this rule reads. `disallowed-types` and
/// `disallowed-macros` carry no entry today; add them here when one lands.
const TABLE: &str = "disallowed-methods";

/// What checking one fence entry concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Resolved,
    /// The first segment names a crate outside the workspace, so there is
    /// nothing here to resolve the path against.
    Skipped,
    Unresolved,
}

/// One `disallowed-methods` entry and what checking it concluded.
pub struct Entry {
    pub path: String,
    /// Where the entry is written, for a report to point at.
    pub location: String,
    pub state: State,
}

/// One crate's surface: the path an importer writes for each exported item,
/// mapped to the names reachable through it.
type Surface = BTreeMap<String, Vec<String>>;

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    Ok(entries(ws)?
        .into_iter()
        .filter(|entry| entry.state == State::Unresolved)
        .map(|entry| {
            Violation::at(
                RULE,
                entry.path.clone(),
                format!(
                    "`{TABLE}` path `{}` reaches no exported item, so clippy silently enforces nothing",
                    entry.path
                ),
                entry.location,
            )
        })
        .collect())
}

/// Every fence entry with its state, including the ones this rule skips.
///
/// `arch-report` prints these so a skip is visible rather than silent.
pub fn entries(ws: &Workspace) -> Result<Vec<Entry>> {
    let file = ws.root.join("clippy.toml");
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let table: toml::Table = text.parse().context("parsing clippy.toml")?;
    let paths = fenced_paths(&table);
    let surfaces = surfaces(ws, &paths)?;
    let external = external(ws);
    Ok(paths
        .into_iter()
        .map(|path| Entry {
            location: location(&text, &path),
            state: judge(&path, &surfaces, &external),
            path,
        })
        .collect())
}

/// Crate identifiers a fence path may name without the workspace holding its
/// source: the standard library, and every dependency any member declares under
/// the name code writes for it.
///
/// A member is in here too, as its dependents' declaration — harmless, since
/// [`judge`] asks `surfaces` first and only falls back here.
fn external(ws: &Workspace) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = ["std", "core", "alloc", "proc_macro"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    for member in ws.members() {
        out.extend(
            member
                .dependencies
                .iter()
                .map(|dep| dep.alias.replace('-', "_")),
        );
    }
    out
}

/// Whether a fence path reaches an item the workspace exports.
///
/// Two shapes resolve: the whole path naming a free item, and all but the last
/// segment naming a type or trait that holds the last one. A first segment in
/// `external` is skipped; one in neither map named a member since renamed or
/// dropped, and a path with no `::` reaches nothing, so both are reported.
fn judge(path: &str, surfaces: &BTreeMap<String, Surface>, external: &BTreeSet<String>) -> State {
    // A single segment names no crate, so clippy resolves it to nothing either
    // and the entry fences nothing. Reported rather than skipped.
    let Some((krate, rest)) = path.split_once("::") else {
        return State::Unresolved;
    };
    let Some(items) = surfaces.get(krate) else {
        return if external.contains(krate) {
            State::Skipped
        } else {
            State::Unresolved
        };
    };
    if items.contains_key(rest) {
        return State::Resolved;
    }
    let holds = rest.rsplit_once("::").is_some_and(|(owner, member)| {
        items
            .get(owner)
            .is_some_and(|names| names.iter().any(|name| name == member))
    });
    if holds {
        State::Resolved
    } else {
        State::Unresolved
    }
}

/// Every path in the fence table, written bare or as `{ path = … }`.
fn fenced_paths(table: &toml::Table) -> Vec<String> {
    table
        .get(TABLE)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            toml::Value::String(path) => Some(path.clone()),
            toml::Value::Table(fields) => fields
                .get("path")
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect()
}

/// The exported surface of each workspace crate some fence path names.
///
/// Only those crates are read: parsing a crate nothing fences answers nothing.
fn surfaces(ws: &Workspace, paths: &[String]) -> Result<BTreeMap<String, Surface>> {
    let fenced: BTreeSet<&str> = paths
        .iter()
        .filter_map(|path| path.split_once("::"))
        .map(|(krate, _)| krate)
        .collect();
    let mut out = BTreeMap::new();
    for member in ws.members() {
        let ident = crate_ident(member);
        if !fenced.contains(ident.as_str()) {
            continue;
        }
        let Some(target) = member.targets.iter().find(|t| t.library()) else {
            continue;
        };
        let sources: BTreeMap<String, String> = ws.sources(member)?.into_iter().collect();
        let mut surface = Surface::new();
        for export in exports(&sources, &ws.relative(&target.path))? {
            surface
                .entry(export.written())
                .or_default()
                .extend(export.associated);
        }
        out.insert(ident, surface);
    }
    Ok(out)
}

/// The identifier a path writes for a member: its `[lib] name` where one is
/// set, and otherwise the package name with dashes replaced, as cargo does.
fn crate_ident(member: &Member) -> String {
    member
        .manifest
        .get("lib")
        .and_then(|lib| lib.get("name"))
        .and_then(toml::Value::as_str)
        .map_or_else(|| member.name.replace('-', "_"), str::to_owned)
}

fn location(text: &str, path: &str) -> String {
    // Matched quoted: a bare `contains` also hits a longer path that holds this
    // one, and sends the reader to another entry's line.
    let quoted = format!("\"{path}\"");
    text.lines()
        .position(|line| line.contains(&quoted))
        .map_or_else(
            || "clippy.toml".to_string(),
            |i| format!("clippy.toml:{}", i + 1),
        )
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture manifests must parse")]
mod tests {
    use super::*;

    /// A surface from (crate, written path, associated names) triples.
    fn surfaces(entries: &[(&str, &str, &[&str])]) -> BTreeMap<String, Surface> {
        let mut out: BTreeMap<String, Surface> = BTreeMap::new();
        for (krate, written, associated) in entries {
            out.entry((*krate).to_string()).or_default().insert(
                (*written).to_string(),
                associated.iter().map(|n| (*n).to_string()).collect(),
            );
        }
        out
    }

    /// Crate names the fixture workspace can reach without holding the source.
    fn deps() -> BTreeSet<String> {
        ["std", "tokio"].into_iter().map(str::to_owned).collect()
    }

    fn types() -> BTreeMap<String, Surface> {
        surfaces(&[
            (
                "moatless_types",
                "access_control::ScopeContext",
                &["system", "for_user"],
            ),
            ("moatless_types", "ScopeContext", &["system", "for_user"]),
            ("moatless_types", "access_control::normalise", &[]),
        ])
    }

    #[test]
    fn a_path_to_an_inherent_method_resolves() {
        let found = judge(
            "moatless_types::access_control::ScopeContext::system",
            &types(),
            &deps(),
        );
        assert_eq!(found, State::Resolved);
    }

    /// The finding this rule exists for: the type survives the rename, so a
    /// check that stopped at the type would stay green through it.
    #[test]
    fn renaming_the_method_leaves_the_type_resolving_and_the_path_not() {
        let renamed = surfaces(&[(
            "moatless_types",
            "access_control::ScopeContext",
            &["unscoped"],
        )]);
        assert_eq!(
            judge(
                "moatless_types::access_control::ScopeContext::system",
                &renamed,
                &deps(),
            ),
            State::Unresolved
        );
        assert_eq!(
            judge(
                "moatless_types::access_control::ScopeContext::unscoped",
                &renamed,
                &deps(),
            ),
            State::Resolved
        );
    }

    #[test]
    fn a_path_through_a_re_export_resolves() {
        assert_eq!(
            judge("moatless_types::ScopeContext::system", &types(), &deps()),
            State::Resolved
        );
    }

    #[test]
    fn a_free_function_resolves_on_the_whole_path() {
        assert_eq!(
            judge(
                "moatless_types::access_control::normalise",
                &types(),
                &deps()
            ),
            State::Resolved
        );
    }

    #[test]
    fn a_moved_type_no_longer_resolves() {
        assert_eq!(
            judge(
                "moatless_types::scope::ScopeContext::system",
                &types(),
                &deps()
            ),
            State::Unresolved
        );
    }

    #[test]
    fn a_path_naming_no_workspace_crate_is_skipped() {
        for path in ["std::env::var", "tokio::spawn"] {
            assert_eq!(judge(path, &types(), &deps()), State::Skipped, "{path}");
        }
    }

    #[test]
    fn a_path_with_no_crate_segment_does_not_resolve() {
        assert_eq!(judge("Utc", &types(), &deps()), State::Unresolved);
    }

    /// Renaming or dropping the crate takes its entry out of both maps, which
    /// disarms the fence as silently as renaming the method.
    #[test]
    fn a_first_segment_naming_neither_a_member_nor_a_dependency_does_not_resolve() {
        assert_eq!(
            judge(
                "moatless_kinds::access_control::ScopeContext::system",
                &types(),
                &deps(),
            ),
            State::Unresolved
        );
    }

    #[test]
    fn a_bare_crate_name_with_no_item_does_not_resolve() {
        assert_eq!(
            judge("moatless_types::Missing", &types(), &deps()),
            State::Unresolved
        );
    }

    #[test]
    fn a_fence_entry_is_read_whether_written_bare_or_as_a_table() {
        let table: toml::Table = r#"
            disallowed-methods = [
                "std::env::var",
                { path = "std::fs::read", reason = "why" },
            ]
        "#
        .parse()
        .expect("the fixture parses");
        assert_eq!(
            fenced_paths(&table),
            vec!["std::env::var".to_string(), "std::fs::read".to_string()]
        );
    }

    #[test]
    fn no_fence_table_means_no_entries() {
        let table: toml::Table = "doc-valid-idents = [\"OTel\"]".parse().expect("parses");
        assert!(fenced_paths(&table).is_empty());
    }

    #[test]
    fn the_location_names_the_line_the_entry_sits_on() {
        let text =
            "# note\ndisallowed-methods = [\n    { path = \"a::b::c\" },\n    \"a::b\",\n]\n";
        assert_eq!(location(text, "a::b::c"), "clippy.toml:3");
        // `a::b` is a substring of line 3; the quoted form sends it to its own.
        assert_eq!(location(text, "a::b"), "clippy.toml:4");
        assert_eq!(location(text, "x::y"), "clippy.toml");
    }
}

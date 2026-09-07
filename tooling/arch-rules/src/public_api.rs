//! What a crate exports, as opposed to what sits under its `src/`.
//!
//! The walk starts at the crate root, so a `pub` item in a private module or
//! behind a test-support `cfg` is not counted as exported. Re-exports are
//! followed one step, under the name an importer writes — under `as`, only the
//! alias.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};

/// A type another crate can name, and the file that defines it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Export {
    pub name: String,
    pub file: String,
}

/// Every type reachable from `root` through public modules and re-exports.
pub fn exports(sources: &BTreeMap<String, String>, root: &str) -> Result<Vec<Export>> {
    let mut queue = VecDeque::from([(root.to_string(), Vec::new(), true)]);
    let mut visited = BTreeSet::new();

    // Types with where they sit, and re-exports as written. Both are judged
    // after the walk: a `pub use` may name a type the walk has not reached
    // yet, and resolving its first segment needs every module to be known.
    let mut candidates: Vec<(Export, Vec<String>, bool)> = Vec::new();
    let mut written: Vec<Written> = Vec::new();
    let mut modules: BTreeSet<Vec<String>> = BTreeSet::new();

    while let Some((file, module, reachable)) = queue.pop_front() {
        if !visited.insert(file.clone()) {
            continue;
        }
        let Some(text) = sources.get(&file) else {
            continue;
        };
        let parsed = syn::parse_file(text).with_context(|| format!("parsing {file}"))?;
        if crate::source_index::test_only(&parsed.attrs) {
            continue;
        }
        let mut found = Scanned::default();
        scan(&parsed.items, &module, reachable, &mut found);

        modules.insert(module.clone());
        modules.extend(found.inline.iter().cloned());

        for (name, at, public) in found.types {
            candidates.push((
                Export {
                    name,
                    file: file.clone(),
                },
                at,
                public,
            ));
        }

        for (name, public) in found.mods {
            let mut at = module.clone();
            at.push(name.clone());
            // Recorded whether or not the file exists, so a `use` naming it
            // still resolves as local.
            modules.insert(at.clone());
            if let Some(child) = resolve(sources, &file, &name, file == root) {
                queue.push_back((child, at, reachable && public));
            }
        }

        // A `pub use` inside a private module re-exports nothing outward.
        if reachable {
            for (at, tree) in found.uses {
                let mut prefix = Vec::new();
                walk_use(&tree, &mut prefix, &at, &mut written);
            }
        }
    }

    let mut reexports: Vec<ReExport> = Vec::new();
    let mut globbed: BTreeSet<Vec<String>> = BTreeSet::new();
    for use_ in written {
        let Some(module) = local_path(&use_.prefix, &use_.at, &modules) else {
            continue;
        };
        match use_.item {
            Some((name, alias)) => reexports.push(ReExport {
                module,
                name,
                alias,
            }),
            None => {
                globbed.insert(module);
            }
        }
    }

    let mut out = BTreeSet::new();
    for (export, at, public) in candidates {
        // A type is exported under every name an importer may write for it:
        // its own where the module chain is public, and the alias of each
        // `pub use` that names it — which is the only name a rename gives.
        if public || globbed.iter().any(|glob| at.starts_with(glob)) {
            out.insert(export.clone());
        }
        for lifted in reexports.iter().filter(|r| r.names(&export.name, &at)) {
            out.insert(Export {
                name: lifted.alias.clone(),
                file: export.file.clone(),
            });
        }
    }
    Ok(out.into_iter().collect())
}

/// A `pub use` as written, before its first segment is resolved.
struct Written {
    /// The module the `use` was written in.
    at: Vec<String>,
    /// The path segments before the leaf.
    prefix: Vec<String>,
    /// The item's name and the name it is given, or `None` for a glob.
    item: Option<(String, String)>,
}

/// A `pub use` of one item, and the name it gives that item.
struct ReExport {
    /// The module inside this crate it points into. A path naming another
    /// crate re-exports nothing of ours.
    module: Vec<String>,
    /// The item's own name.
    name: String,
    /// What an importer writes, which differs under `as`.
    alias: String,
}

impl ReExport {
    /// Whether this re-export is the one that lifts a type named `name` out of
    /// module `at`.
    ///
    /// The module is matched as a prefix so a `pub use` pointing at an
    /// ancestor still lifts, which is how a re-export chain reads from the
    /// outside.
    fn names(&self, name: &str, at: &[String]) -> bool {
        self.name == name && at.starts_with(&self.module)
    }
}

#[derive(Default)]
struct Scanned {
    /// Type name, the module path it sits at, and whether every module
    /// enclosing it within this file is public.
    types: Vec<(String, Vec<String>, bool)>,
    /// A child module in another file, and whether it is declared `pub`.
    mods: Vec<(String, bool)>,
    /// Module paths declared inline in this file, at any visibility.
    inline: Vec<Vec<String>>,
    /// A `pub use`, with the module path it was written in.
    uses: Vec<(Vec<String>, syn::UseTree)>,
}

fn scan(items: &[syn::Item], module: &[String], public: bool, out: &mut Scanned) {
    for item in items {
        match item {
            syn::Item::Struct(s) if is_pub(&s.vis) && !crate::source_index::test_only(&s.attrs) => {
                out.types
                    .push((s.ident.to_string(), module.to_vec(), public));
            }
            syn::Item::Enum(e) if is_pub(&e.vis) && !crate::source_index::test_only(&e.attrs) => {
                out.types
                    .push((e.ident.to_string(), module.to_vec(), public));
            }
            syn::Item::Mod(m) if !crate::source_index::test_only(&m.attrs) => match &m.content {
                Some((_, nested)) => {
                    let mut inner = module.to_vec();
                    inner.push(m.ident.to_string());
                    out.inline.push(inner.clone());
                    scan(nested, &inner, public && is_pub(&m.vis), out);
                }
                None => out.mods.push((m.ident.to_string(), is_pub(&m.vis))),
            },
            syn::Item::Use(u) if is_pub(&u.vis) && !crate::source_index::test_only(&u.attrs) => {
                out.uses.push((module.to_vec(), u.tree.clone()));
            }
            _ => {}
        }
    }
}

/// Record each `pub use` leaf as written, for resolution after the walk.
fn walk_use(tree: &syn::UseTree, prefix: &mut Vec<String>, at: &[String], out: &mut Vec<Written>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            walk_use(&p.tree, prefix, at, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let name = n.ident.to_string();
            out.push(Written {
                at: at.to_vec(),
                prefix: prefix.clone(),
                item: Some((name.clone(), name)),
            });
        }
        syn::UseTree::Rename(r) => {
            // The alias only. `pub use internal::Original as Alias` leaves
            // `Original` unnameable from outside, so recording it would both
            // report a collision that cannot happen and miss one on `Alias`.
            out.push(Written {
                at: at.to_vec(),
                prefix: prefix.clone(),
                item: Some((r.ident.to_string(), r.rename.to_string())),
            });
        }
        syn::UseTree::Glob(_) => {
            out.push(Written {
                at: at.to_vec(),
                prefix: prefix.clone(),
                item: None,
            });
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                walk_use(item, prefix, at, out);
            }
        }
    }
}

/// A use-path as a module path inside this crate, or `None` when it names
/// another crate.
///
/// `crate`, `self` and `super` are explicit. A bare first segment is a crate
/// name *unless* it is a module in scope — the 2018 edition's uniform paths —
/// so it is resolved against the modules the walk found.
fn local_path(
    prefix: &[String],
    at: &[String],
    modules: &BTreeSet<Vec<String>>,
) -> Option<Vec<String>> {
    let (first, rest) = prefix.split_first()?;
    let mut out = match first.as_str() {
        "crate" => Vec::new(),
        "self" => at.to_vec(),
        "super" => at[..at.len().saturating_sub(1)].to_vec(),
        _ => {
            let mut child = at.to_vec();
            child.push(first.clone());
            if !modules.contains(&child) {
                return None;
            }
            child
        }
    };
    out.extend_from_slice(rest);
    Some(out)
}

/// The file a `mod name;` in `file` refers to, if the crate has one.
fn resolve(
    sources: &BTreeMap<String, String>,
    file: &str,
    name: &str,
    root: bool,
) -> Option<String> {
    let dir = if root {
        Path::new(file).parent()?.to_string_lossy().into_owned()
    } else {
        module_dir(file)?
    };
    [format!("{dir}/{name}.rs"), format!("{dir}/{name}/mod.rs")]
        .into_iter()
        .find(|candidate| sources.contains_key(candidate))
}

/// The directory a file's child modules live in. A root or `mod.rs` owns its
/// own directory; any other file owns the directory named after it.
fn module_dir(file: &str) -> Option<String> {
    let path = Path::new(file);
    let parent = path.parent()?.to_string_lossy().into_owned();
    match path.file_name()?.to_string_lossy().as_ref() {
        "lib.rs" | "main.rs" | "mod.rs" => Some(parent),
        _ => Some(format!("{parent}/{}", path.file_stem()?.to_string_lossy())),
    }
}

fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a fixture that fails to parse is a failing test"
)]
mod tests {
    use super::*;

    fn crate_of(files: &[(&str, &str)]) -> BTreeMap<String, String> {
        files
            .iter()
            .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
            .collect()
    }

    fn names(sources: &BTreeMap<String, String>) -> Vec<String> {
        exports(sources, "src/lib.rs")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    #[test]
    fn a_type_under_a_public_module_chain_is_exported() {
        let sources = crate_of(&[
            ("src/lib.rs", "pub mod task;"),
            ("src/task/mod.rs", "pub mod dao;"),
            ("src/task/dao.rs", "pub struct Row;"),
        ]);
        assert_eq!(names(&sources), vec!["Row"]);
    }

    #[test]
    fn a_type_behind_a_private_module_is_not() {
        let sources = crate_of(&[
            ("src/lib.rs", "mod internal;"),
            ("src/internal.rs", "pub struct Hidden;"),
        ]);
        assert!(names(&sources).is_empty());
    }

    #[test]
    fn a_re_export_lifts_a_type_out_of_a_private_module() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "mod internal;\npub use self::internal::Lifted;",
            ),
            ("src/internal.rs", "pub struct Lifted;\npub struct Left;"),
        ]);
        assert_eq!(names(&sources), vec!["Lifted"]);
    }

    /// An alias is the only name an importer can write, so it is the only
    /// name a collision can happen on.
    #[test]
    fn a_renamed_re_export_exports_the_alias_and_not_the_source_name() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "mod internal;\npub use self::internal::Original as Alias;",
            ),
            ("src/internal.rs", "pub struct Original;"),
        ]);
        assert_eq!(names(&sources), vec!["Alias"]);
    }

    /// Renaming something already public adds a name rather than replacing it:
    /// both are writable.
    #[test]
    fn renaming_a_public_type_leaves_both_names_exported() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "pub mod inner;\npub use self::inner::Original as Alias;",
            ),
            ("src/inner.rs", "pub struct Original;"),
        ]);
        assert_eq!(names(&sources), vec!["Alias", "Original"]);
    }

    #[test]
    fn a_re_export_of_another_crates_type_exports_nothing_of_ours() {
        let sources = crate_of(&[
            ("src/lib.rs", "mod internal;\npub use serde::Serialize;"),
            ("src/internal.rs", "pub struct Serialize;"),
        ]);
        assert!(names(&sources).is_empty());
    }

    /// Uniform paths: a bare first segment naming a child module is local, and
    /// this is the form the workspace uses in `delivery/mod.rs`.
    #[test]
    fn a_bare_path_naming_a_child_module_is_local() {
        let sources = crate_of(&[
            ("src/lib.rs", "pub mod delivery;"),
            (
                "src/delivery/mod.rs",
                "mod admission;\npub use admission::DeliveryAdmission;",
            ),
            (
                "src/delivery/admission.rs",
                "pub struct DeliveryAdmission;\npub struct Unlifted;",
            ),
        ]);
        assert_eq!(names(&sources), vec!["DeliveryAdmission"]);
    }

    #[test]
    fn a_glob_re_export_lifts_the_module_it_names() {
        let sources = crate_of(&[
            ("src/lib.rs", "mod internal;\npub use crate::internal::*;"),
            ("src/internal.rs", "pub struct One;\npub struct Two;"),
        ]);
        assert_eq!(names(&sources), vec!["One", "Two"]);
    }

    #[test]
    fn a_test_support_module_is_not_part_of_the_api() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "#[cfg(any(test, feature = \"test-support\"))]\npub mod test_support;\npub mod real;",
            ),
            ("src/test_support.rs", "pub struct Fake;"),
            ("src/real.rs", "pub struct Real;"),
        ]);
        assert_eq!(names(&sources), vec!["Real"]);
    }

    #[test]
    fn a_feature_gate_that_merely_contains_the_word_test_is_followed() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "#[cfg(feature = \"latest\")]\npub mod newest;",
            ),
            ("src/newest.rs", "pub struct Kept;"),
        ]);
        assert_eq!(names(&sources), vec!["Kept"]);
    }

    #[test]
    fn a_private_inline_module_hides_its_types() {
        let sources = crate_of(&[(
            "src/lib.rs",
            "mod inner { pub struct Hidden; }\npub mod outer { pub struct Shown; }",
        )]);
        assert_eq!(names(&sources), vec!["Shown"]);
    }

    #[test]
    fn a_file_no_module_declares_is_never_read() {
        let sources = crate_of(&[
            ("src/lib.rs", "pub mod real;"),
            ("src/real.rs", "pub struct Real;"),
            ("src/orphan.rs", "pub struct Orphan;"),
        ]);
        assert_eq!(names(&sources), vec!["Real"]);
    }
}

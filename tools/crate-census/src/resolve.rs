//! The backend name resolver: resolve a path used in a backend module to the
//! crate that *defines* the symbol, following `pub use` re-export chains.

use crate::bands::{crate_of_backend_mod, external_crate};
use crate::model::{ModPath, Resolution, Tree};

const MAX_DEPTH: u32 = 40;

pub fn resolve_backend(tree: &Tree, from_mod: &ModPath, segs: &[String], depth: u32) -> Resolution {
    if depth > MAX_DEPTH {
        return Resolution::Unresolved("depth-limit".to_string());
    }
    let Some(head) = segs.first().map(String::as_str) else {
        return Resolution::Unresolved("empty-path".to_string());
    };

    match head {
        "crate" | "$crate" => walk(tree, from_mod, Vec::new(), &segs[1..], depth),
        "self" => walk(tree, from_mod, from_mod.clone(), &segs[1..], depth),
        "super" => {
            let mut n = 0;
            while segs.get(n).map(String::as_str) == Some("super") {
                n += 1;
            }
            let mut base = from_mod.clone();
            for _ in 0..n {
                if base.pop().is_none() {
                    return Resolution::Unresolved("super-above-root".to_string());
                }
            }
            walk(tree, from_mod, base, &segs[n..], depth)
        }
        "Self" => Resolution::Ignore,
        "moatless_backend" => walk(tree, from_mod, Vec::new(), &segs[1..], depth),
        _ => {
            if let Some(c) = external_crate(head) {
                return Resolution::ExternalCrate(c);
            }
            resolve_bare(tree, from_mod, segs, depth)
        }
    }
}

/// A bare (non-anchored, non-known-crate) head: a `use` alias, a child module, a
/// local item, a name brought in by an internal glob, or — the common case for a
/// multi-segment path — an external crate name (`std`, `serde`, `axum`, …).
fn resolve_bare(tree: &Tree, from_mod: &ModPath, segs: &[String], depth: u32) -> Resolution {
    let head = segs[0].as_str();

    // 1. `use` alias in scope. A self-referential import (`use async_trait::async_trait`,
    //    where the alias equals the path's crate head) names the external crate,
    //    not a re-aliased path — fall through and let it resolve as external / Ignore.
    if let Some(target) = tree.uses.get(from_mod).and_then(|u| u.get(head)) {
        if target.first().map(String::as_str) != Some(head) {
            let mut combined = target.clone();
            combined.extend_from_slice(&segs[1..]);
            return resolve_backend(tree, from_mod, &combined, depth + 1);
        }
    }

    // 2. Child module of the current module.
    if tree.has_child(from_mod, head) {
        return walk(tree, from_mod, from_mod.clone(), segs, depth);
    }

    // 3. Item defined locally -> same crate, no edge.
    if tree.defines(from_mod, head) {
        return Resolution::Ignore;
    }

    // 4. Internal globs (`use super::*`, `use crate::x::*`) that actually export
    //    the head. This is the facade-glob audit: two internal globs that both
    //    could export the head are ambiguous rather than silently guessed.
    let mut glob_hits: Vec<Resolution> = Vec::new();
    if let Some(globs) = tree.glob_uses.get(from_mod) {
        for prefix in globs {
            let anchored = prefix
                .first()
                .is_some_and(|h| {
                    matches!(
                        h.as_str(),
                        "crate" | "super" | "self" | "moatless_backend"
                    )
                });
            if !anchored {
                continue; // external glob — only brings floor/std names (legal or ignore)
            }
            if let Resolution::Internal(_, ref m) = resolve_backend(tree, from_mod, prefix, depth + 1) {
                let exports = tree.defines(m, head)
                    || tree.reexports.get(m).is_some_and(|r| r.contains_key(head));
                if exports {
                    let mut combined = prefix.clone();
                    combined.push(head.to_string());
                    combined.extend_from_slice(&segs[1..]);
                    glob_hits.push(resolve_backend(tree, from_mod, &combined, depth + 1));
                }
            }
        }
    }
    match glob_hits.len() {
        0 => Resolution::Ignore, // external crate (std / third-party) or floor via external glob
        1 => glob_hits.into_iter().next().unwrap_or(Resolution::Ignore),
        _ => Resolution::Unresolved("glob-through-facade-ambiguous".to_string()),
    }
}

/// Walk `rest` from module `start`, descending into child modules and following
/// `pub use` re-exports (facade laundering) to the definition crate.
fn walk(tree: &Tree, from_mod: &ModPath, start: ModPath, rest: &[String], depth: u32) -> Resolution {
    let mut cur = start;
    let mut i = 0;
    while i < rest.len() {
        let s = rest[i].as_str();

        if tree.has_child(&cur, s) {
            cur.push(s.to_string());
            i += 1;
            continue;
        }

        // Named re-export: follow to the real definition.
        if let Some(target) = tree.reexports.get(&cur).and_then(|r| r.get(s)) {
            let mut combined = target.clone();
            combined.extend_from_slice(&rest[i + 1..]);
            return resolve_backend(tree, &cur, &combined, depth + 1);
        }

        // A local definition shadows every glob re-export, so attribute it to
        // `cur`'s own band here — before the glob arm below, which cannot see a
        // floor crate's exports and would otherwise launder the name to it.
        if tree.defines(&cur, s) {
            return Resolution::Internal(crate_of_backend_mod(&cur).edge_crate().to_string(), cur);
        }

        // Glob re-export: try each; a definite single crate wins, 2+ is ambiguous.
        if let Some(globs) = tree.glob_reexports.get(&cur) {
            let mut hits: Vec<Resolution> = Vec::new();
            for g in globs {
                match resolve_backend(tree, &cur, g, depth + 1) {
                    Resolution::Internal(_, ref m) => {
                        let exports = tree.defines(m, s)
                            || tree.reexports.get(m).is_some_and(|r| r.contains_key(s));
                        if exports {
                            let mut combined = g.clone();
                            combined.push(s.to_string());
                            combined.extend_from_slice(&rest[i + 1..]);
                            hits.push(resolve_backend(tree, &cur, &combined, depth + 1));
                        }
                    }
                    // A glob re-export of an external crate (`pub use moatless_x::*`)
                    // means this symbol is defined in that crate.
                    Resolution::ExternalCrate(c) => hits.push(Resolution::ExternalCrate(c)),
                    _ => {}
                }
            }
            match hits.len() {
                1 => return hits.into_iter().next().unwrap_or(Resolution::Ignore),
                n if n > 1 => {
                    // If every hit is the same crate, it is not ambiguous.
                    if all_same_crate(&hits) {
                        return hits.into_iter().next().unwrap_or(Resolution::Ignore);
                    }
                    return Resolution::Unresolved("glob-reexport-ambiguous".to_string());
                }
                _ => {}
            }
        }

        // `s` is an item (or associated item) defined in `cur`.
        let _ = from_mod;
        return Resolution::Internal(crate_of_backend_mod(&cur).edge_crate().to_string(), cur);
    }

    // Path names the module `cur` itself.
    Resolution::Internal(crate_of_backend_mod(&cur).edge_crate().to_string(), cur)
}

fn all_same_crate(hits: &[Resolution]) -> bool {
    let mut seen: Option<&str> = None;
    for h in hits {
        let c = match h {
            Resolution::Internal(c, _) | Resolution::ExternalCrate(c) => c.as_str(),
            _ => return false,
        };
        match seen {
            None => seen = Some(c),
            Some(prev) if prev != c => return false,
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tree;

    fn m(parts: &[&str]) -> ModPath {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    fn describe(r: &Resolution) -> String {
        match r {
            Resolution::Internal(c, module) => format!("Internal({c}, {module:?})"),
            Resolution::ExternalCrate(c) => format!("ExternalCrate({c})"),
            Resolution::Ignore => "Ignore".to_string(),
            Resolution::Unresolved(x) => format!("Unresolved({x})"),
        }
    }

    /// A local definition in a module that also glob-re-exports a floor crate must
    /// attribute to its own band, while a name that only comes through the floor
    /// glob must still attribute to the floor.
    #[test]
    fn a_local_def_shadows_a_floor_glob_reexport() {
        let mut tree = Tree::default();
        // crate -> bindings -> schemas, and crate -> task -> presenter (the caller).
        tree.children
            .entry(m(&[]))
            .or_default()
            .extend(["bindings".to_string(), "task".to_string()]);
        tree.children
            .entry(m(&["bindings"]))
            .or_default()
            .insert("schemas".to_string());
        tree.children
            .entry(m(&["task"]))
            .or_default()
            .insert("presenter".to_string());
        // schemas glob-re-exports the api-schemas floor crate AND defines its own fn.
        tree.glob_reexports
            .entry(m(&["bindings", "schemas"]))
            .or_default()
            .push(m(&["moatless_api_schemas", "bindings"]));
        tree.defined
            .entry(m(&["bindings", "schemas"]))
            .or_default()
            .insert("task_binding_summary_from_binding".to_string());

        let from = m(&["task", "presenter"]);

        // The locally-defined fn must attribute to its own (loops) band, not floor.
        let local = ["crate", "bindings", "schemas", "task_binding_summary_from_binding"]
            .map(String::from);
        match resolve_backend(&tree, &from, &local, 0) {
            Resolution::Internal(c, module) => {
                assert_eq!(c, "loops", "a local def must attribute to its own band");
                assert_eq!(module, m(&["bindings", "schemas"]));
            }
            other => panic!("local def must resolve Internal(loops); got {}", describe(&other)),
        }

        // A name that is NOT defined locally really does come from the floor glob;
        // the fix must not over-reach and steal it back to the module's band.
        let via_glob = ["crate", "bindings", "schemas", "TaskBindingSummary"].map(String::from);
        match resolve_backend(&tree, &from, &via_glob, 0) {
            Resolution::ExternalCrate(c) => {
                assert_eq!(c, "floor:api-schemas", "a genuine floor re-export must stay floor");
            }
            other => panic!(
                "a glob-only name must resolve ExternalCrate(floor:api-schemas); got {}",
                describe(&other)
            ),
        }
    }
}

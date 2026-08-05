//! Data model: the backend module tree, resolution outcomes, and classified
//! reference records.

use std::collections::{HashMap, HashSet};

use crate::bands::band_of;
use crate::RawRef;

pub type ModPath = Vec<String>;

/// The backend module tree plus per-module resolution tables, built in pass 1.
#[derive(Default)]
pub struct Tree {
    /// Every module path that exists (file-based or inline).
    pub modules: HashSet<ModPath>,
    /// Child *module* names per module (not items).
    pub children: HashMap<ModPath, HashSet<String>>,
    /// `use` imports: alias -> raw path segments (with their own anchor head).
    pub uses: HashMap<ModPath, HashMap<String, Vec<String>>>,
    /// `use foo::*;` glob imports: list of raw path prefixes.
    pub glob_uses: HashMap<ModPath, Vec<Vec<String>>>,
    /// `pub use` re-exports: name -> raw path segments.
    pub reexports: HashMap<ModPath, HashMap<String, Vec<String>>>,
    /// `pub use foo::*;` glob re-exports.
    pub glob_reexports: HashMap<ModPath, Vec<Vec<String>>>,
    /// Items (struct/enum/fn/trait/type/const/static/macro) defined directly.
    pub defined: HashMap<ModPath, HashSet<String>>,
    /// Module paths declared `#[cfg(test)] mod ...;` — including file-modules
    /// whose body lives in a separate file that `discover()` parses on its own.
    /// A reference in such a module (or any descendant) is test-context, not a
    /// band-bound production edge.
    pub cfg_test_mods: HashSet<ModPath>,
}

impl Tree {
    /// Is `m` inside a `#[cfg(test)]` module (itself or any ancestor)?
    pub fn under_cfg_test(&self, m: &ModPath) -> bool {
        let mut cur = m.clone();
        loop {
            if self.cfg_test_mods.contains(&cur) {
                return true;
            }
            if cur.pop().is_none() {
                return false;
            }
        }
    }
}

impl Tree {
    pub fn has_child(&self, m: &ModPath, name: &str) -> bool {
        self.children.get(m).is_some_and(|c| c.contains(name))
    }
    pub fn defines(&self, m: &ModPath, name: &str) -> bool {
        self.defined.get(m).is_some_and(|d| d.contains(name))
    }
}

/// Where a path resolves.
pub enum Resolution {
    /// Definition lives in a backend target crate, at the given definition module.
    Internal(String, ModPath),
    /// Definition lives in an existing workspace crate (floor or adapter-model).
    ExternalCrate(String),
    /// std / third-party / local item / `Self` — no census edge.
    Ignore,
    /// Could not be resolved (glob-through-facade, prelude, or macro variable).
    Unresolved(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefClass {
    /// A typed path in expression or type position, or a `use` leaf.
    Code,
    /// A path inside an attribute token stream (`#[utoipa::path(...)]`, derives).
    Attribute,
    /// A path inside a function-like macro invocation's token stream.
    Macro,
    /// A path inside a doc comment (`///` / `//!`). Never a Cargo edge.
    Doc,
}

impl RefClass {
    pub fn tag(self) -> &'static str {
        match self {
            RefClass::Code => "CODE",
            RefClass::Attribute => "ATTR",
            RefClass::Macro => "MACRO",
            RefClass::Doc => "DOC",
        }
    }
}

/// A classified cross-crate reference.
pub struct RefRecord {
    pub source_crate: String,
    pub target_crate: String,
    pub class: RefClass,
    pub file: String,
    pub line: usize,
    pub path_text: String,
    pub illegal: bool,
    pub test_context: bool,
    pub to_floor: bool,
    pub unresolved: bool,
}

impl RefRecord {
    pub fn edge(r: &RawRef, target: String) -> Self {
        // `illegal` records that the reference points upward/sideways across the
        // band order. For a code/attr/macro reference that is an illegal Cargo
        // edge; for a Doc reference it is a would-be-illegal intra-doc link (real
        // carve work per Step 8f, but never a Cargo edge). The report and the
        // baseline keep the two apart — a doc path is never in the illegal total.
        let illegal = classify_edge(&r.source_crate, &target);
        let test_context = matches!(
            r.scope,
            crate::bands::ScopeKind::BackendTest | crate::bands::ScopeKind::FloorTest
        ) || r.in_test_cfg;
        RefRecord {
            to_floor: target.starts_with("floor:"),
            source_crate: r.source_crate.clone(),
            target_crate: target,
            class: r.class,
            file: r.file.clone(),
            line: r.line,
            path_text: r.segs.join("::"),
            illegal,
            test_context,
            unresolved: false,
        }
    }

    pub fn unresolved(r: &RawRef, reason: String) -> Self {
        RefRecord {
            source_crate: r.source_crate.clone(),
            target_crate: format!("?{reason}"),
            class: r.class,
            file: r.file.clone(),
            line: r.line,
            path_text: r.segs.join("::"),
            illegal: false,
            test_context: false,
            to_floor: false,
            unresolved: true,
        }
    }
}

/// A cross-crate edge is legal iff it runs from a higher band to a lower one.
/// Illegal = upward (`band(src) < band(dst)`) or sibling (same band, distinct
/// crate). Returns `true` when the edge is illegal.
///
/// The existing `crates/*` floor crates are excluded on either side: they are
/// real crates whose dependency order Cargo already enforces, so a `floor -> floor`
/// edge is a legal, existing dependency and a `target -> floor` edge is downward
/// by construction. The census scores only the 15 target crates carved from
/// `backend/src`.
pub fn classify_edge(src: &str, dst: &str) -> bool {
    if src == dst {
        return false;
    }
    if src.starts_with("floor:") || dst.starts_with("floor:") {
        return false;
    }
    let (sb, db) = (band_of(src), band_of(dst));
    sb <= db
}

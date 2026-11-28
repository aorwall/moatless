//! What a crate exports, as opposed to what sits under its `src/`.
//!
//! The walk starts at the crate root, so a `pub` item in a private module or
//! behind a test-support `cfg` is not counted as exported. A re-export chain is
//! followed to its end, under the name an importer writes — under `as`, only
//! the alias. [`Scope`] decides whether the modules a `test-support` build adds
//! count as exported.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};
use syn::visit::Visit;

/// The kind of declaration an export names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Struct,
    Enum,
    Trait,
    /// A `type` alias.
    Alias,
    Function,
    /// A `const` or a `static`.
    Value,
}

/// An item another crate can name, and the file that defines it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Export {
    pub name: String,
    pub file: String,
    pub kind: Kind,
    /// The module path an importer writes to reach it, below the crate root.
    pub path: Vec<String>,
    /// Type names a declaration exposes: `pub` fields, enum variant fields,
    /// function args/returns, trait signatures, and `pub` inherent methods.
    /// Bodies are not read.
    pub signature: Vec<String>,
    /// Names of the items reached through this one: its `pub` inherent methods,
    /// consts and associated types, and every item a `pub` trait declares. This
    /// is the trailing segment of a path like `ScopeContext::system`, which
    /// [`Self::signature`] never carries.
    pub associated: Vec<String>,
}

impl Export {
    /// The path an importer writes, joined with `::`.
    pub fn written(&self) -> String {
        let mut parts = self.path.clone();
        parts.push(self.name.clone());
        parts.join("::")
    }
}

/// Which build the walk counts items from.
///
/// A consumer enabling `test-support` or `db-tests` can name what those
/// features add, so a privacy rule asks both; `#[cfg(test)]` alone is never
/// nameable from another crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Default,
    TestSupport,
}

impl Scope {
    fn hides(self, attrs: &[syn::Attribute]) -> bool {
        let gated = crate::source_index::test_only(attrs);
        match self {
            Self::Default => gated,
            Self::TestSupport => gated && !enabled_by_feature(attrs),
        }
    }
}

/// Every item reachable from `root` through public modules and re-exports.
pub fn exports(sources: &BTreeMap<String, String>, root: &str) -> Result<Vec<Export>> {
    exports_in(sources, root, Scope::Default)
}

/// Every item reachable from `root` in one build scope.
pub fn exports_in(
    sources: &BTreeMap<String, String>,
    root: &str,
    scope: Scope,
) -> Result<Vec<Export>> {
    let mut queue = VecDeque::from([(root.to_string(), Vec::new(), true)]);
    let mut visited = BTreeSet::new();

    // Types with where they sit, and re-exports as written. Both are judged
    // after the walk: a `pub use` may name a type the walk has not reached
    // yet, and resolving its first segment needs every module to be known.
    let mut candidates: Vec<(Export, Vec<String>, bool)> = Vec::new();
    let mut written: Vec<Written> = Vec::new();
    let mut modules: BTreeSet<Vec<String>> = BTreeSet::new();
    // (module, type name) -> the signatures of its inherent `pub` methods, and
    // separately the names of those items.
    let mut inherent: BTreeMap<(Vec<String>, String), Vec<String>> = BTreeMap::new();
    let mut associated: BTreeMap<(Vec<String>, String), Vec<String>> = BTreeMap::new();

    while let Some((file, module, reachable)) = queue.pop_front() {
        if !visited.insert(file.clone()) {
            continue;
        }
        let Some(text) = sources.get(&file) else {
            continue;
        };
        let parsed = syn::parse_file(text).with_context(|| format!("parsing {file}"))?;
        if scope.hides(&parsed.attrs) {
            continue;
        }
        let mut found = Scanned::default();
        scan(&parsed.items, &module, reachable, scope, &mut found);

        modules.insert(module.clone());
        modules.extend(found.inline.iter().cloned());

        for found in found.types {
            candidates.push((
                Export {
                    name: found.name,
                    file: file.clone(),
                    kind: found.kind,
                    path: found.at.clone(),
                    signature: found.signature,
                    associated: found.associated,
                },
                found.at,
                found.public,
            ));
        }

        for block in found.inherent {
            let at = (block.at, block.name);
            inherent
                .entry(at.clone())
                .or_default()
                .extend(block.signature);
            associated.entry(at).or_default().extend(block.associated);
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

        for (at, tree) in found.uses {
            let start = written.len();
            let mut prefix = Vec::new();
            walk_use(&tree, &mut prefix, &at, &mut written);
            for use_ in &mut written[start..] {
                use_.reachable = reachable;
            }
        }
    }

    let mut reexports: Vec<ReExport> = Vec::new();
    let mut globs: Vec<Glob> = Vec::new();
    for use_ in written {
        let Some(module) = local_path(&use_.prefix, &use_.at, &modules) else {
            continue;
        };
        match use_.item {
            Some((name, alias)) => reexports.push(ReExport {
                at: use_.at,
                module,
                name,
                alias,
                reachable: use_.reachable,
            }),
            None => globs.push(Glob {
                at: use_.at,
                module,
                reachable: use_.reachable,
            }),
        }
    }

    let candidates: Vec<(Export, Vec<String>, bool)> = candidates
        .into_iter()
        .map(|(export, at, public)| {
            let signature = merged(&inherent, &at, &export, &export.signature);
            let associated = merged(&associated, &at, &export, &export.associated);
            (
                Export {
                    signature,
                    associated,
                    ..export
                },
                at,
                public,
            )
        })
        .collect();

    let available = reachable_names(&candidates, &reexports, &globs);

    // An item is exported under every name an importer may write for it: its
    // own where the module chain is public, and the name each reachable `pub
    // use` gives it — under `as`, the alias only.
    let mut out: BTreeSet<Export> = candidates
        .into_iter()
        .filter(|(_, _, public)| *public)
        .map(|(export, _, _)| export)
        .collect();
    let lifted = reexports
        .iter()
        .filter(|r| r.reachable)
        .map(|r| &r.at)
        .chain(globs.iter().filter(|g| g.reachable).map(|g| &g.at));
    for at in lifted {
        for (held, export) in available.values().flatten() {
            if held == at {
                out.insert(export.clone());
            }
        }
    }
    Ok(out.into_iter().collect())
}

/// Every name each module holds, keyed by the name an importer writes.
///
/// A `pub use` in a private module is followed, since a chain of re-exports
/// puts the item on the crate surface as surely as one step; the loop runs to a
/// fixed point with duplicates dropped, so mutually re-exporting modules
/// terminate.
fn reachable_names(
    candidates: &[(Export, Vec<String>, bool)],
    reexports: &[ReExport],
    globs: &[Glob],
) -> BTreeMap<String, Vec<(Vec<String>, Export)>> {
    let mut available: BTreeMap<String, Vec<(Vec<String>, Export)>> = BTreeMap::new();
    for (export, at, _) in candidates {
        available
            .entry(export.name.clone())
            .or_default()
            .push((at.clone(), export.clone()));
    }
    loop {
        let mut grew = false;
        for lift in reexports {
            let lifted: Vec<(String, Export)> = available
                .get(&lift.name)
                .into_iter()
                .flatten()
                .filter(|(at, _)| at.starts_with(&lift.module))
                .map(|(_, export)| {
                    (
                        lift.alias.clone(),
                        Export {
                            name: lift.alias.clone(),
                            path: lift.at.clone(),
                            ..export.clone()
                        },
                    )
                })
                .collect();
            grew |= admit(&mut available, &lift.at, lifted);
        }
        for glob in globs {
            let lifted: Vec<(String, Export)> = available
                .iter()
                .flat_map(|(name, held)| held.iter().map(move |entry| (name, entry)))
                .filter(|(_, (at, _))| at.starts_with(&glob.module) && at != &glob.at)
                .map(|(name, (_, export))| {
                    (
                        name.clone(),
                        Export {
                            path: glob.at.clone(),
                            ..export.clone()
                        },
                    )
                })
                .collect();
            grew |= admit(&mut available, &glob.at, lifted);
        }
        if !grew {
            return available;
        }
    }
}

/// Record each name at `at`, and report whether anything was new.
fn admit(
    available: &mut BTreeMap<String, Vec<(Vec<String>, Export)>>,
    at: &[String],
    lifted: Vec<(String, Export)>,
) -> bool {
    let mut grew = false;
    for (name, export) in lifted {
        let slot = available.entry(name).or_default();
        if !slot.iter().any(|(held, e)| held == at && *e == export) {
            slot.push((at.to_vec(), export));
            grew = true;
        }
    }
    grew
}

/// `declared` plus whatever the inherent `impl` blocks beside the type add.
///
/// An inherent block is a separate item from the type it is written for, so its
/// contribution is gathered under `(module, type name)` and joined here.
fn merged(
    from_impls: &BTreeMap<(Vec<String>, String), Vec<String>>,
    at: &[String],
    export: &Export,
    declared: &[String],
) -> Vec<String> {
    let mut out = declared.to_vec();
    if let Some(found) = from_impls.get(&(at.to_vec(), export.name.clone())) {
        out.extend(found.iter().cloned());
    }
    out
}

/// A `pub use` as written, before its first segment is resolved.
struct Written {
    /// The module the `use` was written in.
    at: Vec<String>,
    /// The path segments before the leaf.
    prefix: Vec<String>,
    /// The item's name and the name it is given, or `None` for a glob.
    item: Option<(String, String)>,
    /// Whether every module enclosing the `use` is public.
    reachable: bool,
}

/// A `pub use` of one item, and the name it gives that item.
struct ReExport {
    /// The module the `pub use` was written in, which is where an importer
    /// writes the alias.
    at: Vec<String>,
    /// The module inside this crate it points into. A path naming another crate
    /// re-exports nothing of ours.
    module: Vec<String>,
    /// The item's own name.
    name: String,
    /// What an importer writes, which differs under `as`.
    alias: String,
    /// Whether an importer outside the crate can write `at`.
    reachable: bool,
}

/// A `pub use path::*`, which lifts every name the module holds.
struct Glob {
    at: Vec<String>,
    module: Vec<String>,
    reachable: bool,
}

/// One declaration the scan found, before re-exports are resolved.
struct Declared {
    name: String,
    kind: Kind,
    signature: Vec<String>,
    associated: Vec<String>,
    /// The module path it sits at.
    at: Vec<String>,
    /// Whether every module enclosing it within this file is public.
    public: bool,
}

/// One inherent `impl` block's contribution to the type it is written for.
struct Inherent {
    /// The type the block is written for.
    name: String,
    /// The module the block sits in.
    at: Vec<String>,
    /// Type names in its `pub` items' signatures.
    signature: Vec<String>,
    /// The names of its `pub` items.
    associated: Vec<String>,
}

#[derive(Default)]
struct Scanned {
    types: Vec<Declared>,
    inherent: Vec<Inherent>,
    /// A child module in another file, and whether it is declared `pub`.
    mods: Vec<(String, bool)>,
    /// Module paths declared inline in this file, at any visibility.
    inline: Vec<Vec<String>>,
    /// A `pub use`, with the module path it was written in.
    uses: Vec<(Vec<String>, syn::UseTree)>,
}

fn scan(items: &[syn::Item], module: &[String], public: bool, scope: Scope, out: &mut Scanned) {
    for item in items {
        if scope.hides(crate::source_index::attrs(item)) {
            continue;
        }
        // Name, kind, the type names the declaration exposes, and the names of
        // the items reached through it. Only a trait declares the latter inline;
        // for every other kind they arrive from the inherent blocks below.
        let declared = match item {
            syn::Item::Struct(s) if is_pub(&s.vis) => Some((
                s.ident.to_string(),
                Kind::Struct,
                fields(&s.fields),
                Vec::new(),
            )),
            syn::Item::Enum(e) if is_pub(&e.vis) => Some((
                e.ident.to_string(),
                Kind::Enum,
                e.variants
                    .iter()
                    .flat_map(|v| all_fields(&v.fields))
                    .collect(),
                Vec::new(),
            )),
            syn::Item::Trait(t) if is_pub(&t.vis) => Some((
                t.ident.to_string(),
                Kind::Trait,
                trait_signature(t),
                trait_items(t),
            )),
            syn::Item::Type(t) if is_pub(&t.vis) => {
                Some((t.ident.to_string(), Kind::Alias, named(&t.ty), Vec::new()))
            }
            syn::Item::Fn(f) if is_pub(&f.vis) => Some((
                f.sig.ident.to_string(),
                Kind::Function,
                signature(&f.sig),
                Vec::new(),
            )),
            syn::Item::Const(c) if is_pub(&c.vis) => {
                Some((c.ident.to_string(), Kind::Value, named(&c.ty), Vec::new()))
            }
            syn::Item::Static(s) if is_pub(&s.vis) => {
                Some((s.ident.to_string(), Kind::Value, named(&s.ty), Vec::new()))
            }
            syn::Item::Macro(m) => string_enum_name(m)
                .map(|ident| (ident.to_string(), Kind::Enum, Vec::new(), Vec::new())),
            _ => None,
        };
        if let Some((name, kind, signature, associated)) = declared {
            out.types.push(Declared {
                name,
                kind,
                signature,
                associated,
                at: module.to_vec(),
                public,
            });
        }
        match item {
            syn::Item::Impl(i) if i.trait_.is_none() => {
                if let Some(name) = named(&i.self_ty).first() {
                    out.inherent.push(Inherent {
                        name: name.clone(),
                        at: module.to_vec(),
                        signature: inherent_signature(i, scope),
                        associated: inherent_items(i, scope),
                    });
                }
            }
            syn::Item::Mod(m) => match &m.content {
                Some((_, nested)) => {
                    let mut inner = module.to_vec();
                    inner.push(m.ident.to_string());
                    out.inline.push(inner.clone());
                    scan(nested, &inner, public && is_pub(&m.vis), scope, out);
                }
                None => out.mods.push((m.ident.to_string(), is_pub(&m.vis))),
            },
            syn::Item::Use(u) if is_pub(&u.vis) => {
                out.uses.push((module.to_vec(), u.tree.clone()));
            }
            _ => {}
        }
    }
}

/// Type names in the `pub` fields of a struct, tuple fields included.
fn fields(fields: &syn::Fields) -> Vec<String> {
    fields
        .iter()
        .filter(|f| is_pub(&f.vis))
        .flat_map(|f| named(&f.ty))
        .collect()
}

/// Type names in every field of an enum variant, which are all public.
fn all_fields(fields: &syn::Fields) -> Vec<String> {
    fields.iter().flat_map(|f| named(&f.ty)).collect()
}

fn trait_signature(t: &syn::ItemTrait) -> Vec<String> {
    let mut out: Vec<String> = t.supertraits.iter().flat_map(bound).collect();
    for item in &t.items {
        match item {
            syn::TraitItem::Fn(f) => out.extend(signature(&f.sig)),
            syn::TraitItem::Const(c) => out.extend(named(&c.ty)),
            syn::TraitItem::Type(ty) => out.extend(ty.bounds.iter().flat_map(bound)),
            _ => {}
        }
    }
    out
}

/// The names a trait declares, which a caller writes after `Trait::`.
///
/// A trait item has no visibility of its own — the trait's covers it.
fn trait_items(t: &syn::ItemTrait) -> Vec<String> {
    t.items
        .iter()
        .filter_map(|item| match item {
            syn::TraitItem::Fn(f) => Some(f.sig.ident.to_string()),
            syn::TraitItem::Const(c) => Some(c.ident.to_string()),
            syn::TraitItem::Type(ty) => Some(ty.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// The names an inherent block adds, which a caller writes after `Type::`.
fn inherent_items(i: &syn::ItemImpl, scope: Scope) -> Vec<String> {
    i.items
        .iter()
        .filter_map(|item| match item {
            syn::ImplItem::Fn(f) if is_pub(&f.vis) && !scope.hides(&f.attrs) => {
                Some(f.sig.ident.to_string())
            }
            syn::ImplItem::Const(c) if is_pub(&c.vis) && !scope.hides(&c.attrs) => {
                Some(c.ident.to_string())
            }
            syn::ImplItem::Type(ty) if is_pub(&ty.vis) && !scope.hides(&ty.attrs) => {
                Some(ty.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

fn inherent_signature(i: &syn::ItemImpl, scope: Scope) -> Vec<String> {
    i.items
        .iter()
        .filter_map(|item| match item {
            syn::ImplItem::Fn(f) if is_pub(&f.vis) && !scope.hides(&f.attrs) => {
                Some(signature(&f.sig))
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn bound(b: &syn::TypeParamBound) -> Vec<String> {
    let mut found = Types::default();
    found.visit_type_param_bound(b);
    found.names
}

fn signature(sig: &syn::Signature) -> Vec<String> {
    let mut found = Types::default();
    found.visit_signature(sig);
    found.names
}

/// The last segment of every path a type writes, outermost first.
pub(crate) fn named(ty: &syn::Type) -> Vec<String> {
    let mut found = Types::default();
    found.visit_type(ty);
    found.names
}

/// The last segment of every path a declaration writes, with bodies skipped.
#[derive(Default)]
struct Types {
    names: Vec<String>,
}

impl<'ast> Visit<'ast> for Types {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        if let Some(last) = p.segments.last() {
            self.names.push(last.ident.to_string());
        }
        syn::visit::visit_path(self, p);
    }
    fn visit_block(&mut self, _: &'ast syn::Block) {}
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
}

/// Whether a `cfg` names a feature a consumer can turn on to reach the item.
fn enabled_by_feature(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .any(|a| a.parse_args::<syn::Meta>().is_ok_and(|m| support(&m)))
}

fn support(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::NameValue(n) if n.path.is_ident("feature") => {
            if let syn::Expr::Lit(l) = &n.value
                && let syn::Lit::Str(s) = &l.lit
            {
                // `test-support` and `db-tests` are the only two features
                // the workspace declares for a consumer to reach test-only
                // items with; `source_index::test_only` reads the same pair.
                return matches!(s.value().as_str(), "test-support" | "db-tests");
            }
            false
        }
        // Only `any` and `all` carry the feature outward. Under `not`, turning
        // the feature on is what removes the item.
        syn::Meta::List(l) if l.path.is_ident("any") || l.path.is_ident("all") => {
            use syn::parse::Parser;
            let parser = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
            parser
                .parse2(l.tokens.clone())
                .is_ok_and(|children| children.iter().any(support))
        }
        _ => false,
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
                reachable: true,
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
                reachable: true,
            });
        }
        syn::UseTree::Glob(_) => {
            out.push(Written {
                at: at.to_vec(),
                prefix: prefix.clone(),
                item: None,
                reachable: true,
            });
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                walk_use(item, prefix, at, out);
            }
        }
    }
}

/// `None` when the use-path names another crate.
///
/// `crate`, `self`, and `super` are explicit; a bare first segment is a crate
/// name unless it is a module in scope (2018 edition uniform paths), so it is
/// resolved against the modules the walk found.
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

/// The enum name a `string_enum!` invocation declares, since its expansion is
/// unconditionally `pub enum` and never appears as a literal `syn::Item::Enum`.
fn string_enum_name(m: &syn::ItemMacro) -> Option<syn::Ident> {
    use syn::parse::Parser;
    if m.mac.path.segments.last()?.ident != "string_enum" {
        return None;
    }
    // `Parser::parse2` requires the closure to consume the whole stream, so the
    // optional `[extras]` group and the variant body are parsed and discarded
    // too, mirroring `source_index::expand_contract_macro`.
    let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<syn::Ident> {
        input.call(syn::Attribute::parse_outer)?;
        let ident: syn::Ident = input.parse()?;
        if input.peek(syn::token::Bracket) {
            let extras;
            syn::bracketed!(extras in input);
            let _: proc_macro2::TokenStream = extras.parse()?;
        }
        let body;
        syn::braced!(body in input);
        let _: proc_macro2::TokenStream = body.parse()?;
        Ok(ident)
    };
    parser.parse2(m.mac.tokens.clone()).ok()
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

    fn types(sources: &BTreeMap<String, String>) -> Vec<String> {
        exports(sources, "src/lib.rs")
            .unwrap()
            .into_iter()
            .filter(|e| matches!(e.kind, Kind::Struct | Kind::Enum))
            .map(|e| e.name)
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
        assert_eq!(types(&sources), vec!["Row"]);
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
        assert_eq!(types(&sources), vec!["Lifted"]);
    }

    /// An alias is the only name an importer can write, so it is the only name
    /// a collision can happen on.
    #[test]
    fn a_renamed_re_export_exports_the_alias_and_not_the_source_name() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "mod internal;\npub use self::internal::Original as Alias;",
            ),
            ("src/internal.rs", "pub struct Original;"),
        ]);
        assert_eq!(types(&sources), vec!["Alias"]);
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
        assert_eq!(types(&sources), vec!["Alias", "Original"]);
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
        assert_eq!(types(&sources), vec!["DeliveryAdmission"]);
    }

    #[test]
    fn a_glob_re_export_lifts_the_module_it_names() {
        let sources = crate_of(&[
            ("src/lib.rs", "mod internal;\npub use crate::internal::*;"),
            ("src/internal.rs", "pub struct One;\npub struct Two;"),
        ]);
        assert_eq!(types(&sources), vec!["One", "Two"]);
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
        assert_eq!(types(&sources), vec!["Real"]);
    }

    /// The same crate read as a `test-support` consumer sees it: the feature is
    /// one a consumer turns on, so what it adds is nameable.
    #[test]
    fn a_test_support_module_is_part_of_the_test_support_api() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "#[cfg(any(test, feature = \"test-support\"))]\npub mod test_support;\n#[cfg(test)]\npub mod unit;",
            ),
            ("src/test_support.rs", "pub struct Fake;"),
            ("src/unit.rs", "pub struct Local;"),
        ]);
        let found: Vec<String> = exports_in(&sources, "src/lib.rs", Scope::TestSupport)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(found, vec!["Fake"]);
    }

    /// `not(feature = "test-support")` names the feature and is removed by it,
    /// so a `test-support` consumer cannot name what it guards either.
    #[test]
    fn a_negated_support_feature_does_not_open_a_test_only_item() {
        let sources = crate_of(&[
            (
                "src/lib.rs",
                "#[cfg(all(test, not(feature = \"test-support\")))]\npub mod local;",
            ),
            ("src/local.rs", "pub struct Local;"),
        ]);
        assert!(names(&sources).is_empty());
        assert!(
            exports_in(&sources, "src/lib.rs", Scope::TestSupport)
                .unwrap()
                .is_empty()
        );
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
        assert_eq!(types(&sources), vec!["Kept"]);
    }

    #[test]
    fn a_private_inline_module_hides_its_types() {
        let sources = crate_of(&[(
            "src/lib.rs",
            "mod inner { pub struct Hidden; }\npub mod outer { pub struct Shown; }",
        )]);
        assert_eq!(types(&sources), vec!["Shown"]);
    }

    #[test]
    fn a_file_no_module_declares_is_never_read() {
        let sources = crate_of(&[
            ("src/lib.rs", "pub mod real;"),
            ("src/real.rs", "pub struct Real;"),
            ("src/orphan.rs", "pub struct Orphan;"),
        ]);
        assert_eq!(types(&sources), vec!["Real"]);
    }

    #[test]
    fn traits_aliases_functions_and_values_are_exported_with_their_kind() {
        let sources = crate_of(&[(
            "src/lib.rs",
            "pub trait Reader {}\npub type Handle = u8;\npub fn read() {}\npub const LIMIT: u8 = 1;",
        )]);
        let found = exports(&sources, "src/lib.rs").unwrap();
        let kinds: Vec<(String, Kind)> = found.iter().map(|e| (e.name.clone(), e.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("Handle".to_string(), Kind::Alias),
                ("LIMIT".to_string(), Kind::Value),
                ("Reader".to_string(), Kind::Trait),
                ("read".to_string(), Kind::Function),
            ]
        );
    }

    /// A private field hides its type; a `pub` one, tuple field included, does
    /// not. Inherent `pub` methods count as the type's own signature, which is
    /// where a constructor taking a private type shows up.
    #[test]
    fn a_signature_holds_public_fields_arguments_and_inherent_methods() {
        let sources = crate_of(&[(
            "src/lib.rs",
            "pub struct Store { pool: Pool, pub open: Handle }\n\
             pub struct Wrapper(pub Inner);\n\
             impl Store { pub fn new(dao: FileDao) -> Self { todo!() } fn hidden(x: Secret) {} }",
        )]);
        let found = exports(&sources, "src/lib.rs").unwrap();
        let store = found.iter().find(|e| e.name == "Store").unwrap();
        assert!(store.signature.contains(&"Handle".to_string()));
        assert!(store.signature.contains(&"FileDao".to_string()));
        assert!(!store.signature.contains(&"Pool".to_string()));
        assert!(!store.signature.contains(&"Secret".to_string()));
        let wrapper = found.iter().find(|e| e.name == "Wrapper").unwrap();
        assert_eq!(wrapper.signature, vec!["Inner".to_string()]);
    }

    /// `string_enum!` expands to `pub enum $name`, never a literal
    /// `syn::Item::Enum`, so a bare-form and bracket-form invocation both need
    /// their own path to the export list.
    #[test]
    fn a_string_enum_invocation_exports_its_name() {
        let sources = crate_of(&[
            ("src/lib.rs", "pub mod task;"),
            (
                "src/task.rs",
                "crate::string_enum!(\n    CheckoutMode [Default] {\n        #[default]\n        Shared => \"shared\",\n        Isolated => \"isolated\",\n    }\n);\ncrate::string_enum!(\n    Bare { A => \"a\" }\n);",
            ),
        ]);
        assert_eq!(types(&sources), vec!["Bare", "CheckoutMode"]);
    }

    #[test]
    fn a_written_path_is_the_one_an_importer_types() {
        let sources = crate_of(&[
            ("src/lib.rs", "pub mod task;"),
            ("src/task/mod.rs", "pub mod dao;"),
            ("src/task/dao.rs", "pub struct TaskDao;"),
        ]);
        let found = exports(&sources, "src/lib.rs").unwrap();
        assert_eq!(found[0].written(), "task::dao::TaskDao");
    }
}

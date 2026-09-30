//! No member's public API names a vendor SDK type, the owner's included.
//!
//! The owner translates at its boundary: a caller that saw `SdkError` would
//! need the SDK to match on it, which `vendor-crate-owner` forbids. Read are
//! the exported items' signatures, `pub` fields, variant fields, `pub use`
//! targets, `pub` inherent methods, and an exported type's trait impls'
//! trait path and associated types. Bodies are not.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use syn::visit::Visit;

use crate::Violation;
use crate::source_index::Record;
use crate::workspace::Workspace;

pub const RULE: &str = "vendor-type-export";

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let exported: BTreeSet<&str> = index
        .records
        .iter()
        .filter(|r| r.exported)
        .map(|r| r.id.as_str())
        .collect();
    // (package, item, vendor) -> the vendor paths it exposes, and where.
    let mut found: BTreeMap<(String, String, String), (BTreeSet<String>, String)> = BTreeMap::new();
    for record in index.records.iter().filter(|r| !r.test) {
        for (item, path) in surface(record, |p| {
            index
                .resolve(record, p)
                .is_some_and(|id| exported.contains(id.as_str()))
        }) {
            let Some(vendor) = index
                .resolve(record, &path)
                .as_deref()
                .and_then(vendor_crate)
            else {
                continue;
            };
            found
                .entry((record.package.clone(), item, vendor))
                .or_insert_with(|| (BTreeSet::new(), record.location()))
                .0
                .insert(path.join("::"));
        }
    }
    let mut out: Vec<Violation> = found
        .into_iter()
        .map(|((package, item, vendor), (paths, location))| Violation {
            rule: RULE,
            key: format!("{package} {item} -> {vendor}"),
            message: format!(
                "{item} exposes {} from {vendor}: translate it into the owner's own terms",
                paths.into_iter().collect::<Vec<_>>().join(", ")
            ),
            locations: vec![location],
        })
        .collect();
    out.sort();
    Ok(out)
}

/// The vendor crate a resolved path lands in, as its `Cargo.toml` name.
fn vendor_crate(resolved: &str) -> Option<String> {
    let root = resolved.split("::").next()?.replace('_', "-");
    crate::vendor_crate_owner::is_vendor(&root).then_some(root)
}

/// Each path `record` exposes to another crate, with the item that exposes it.
/// `exported_type` says whether an inherent impl's self type is exported.
fn surface(
    record: &Record,
    exported_type: impl Fn(&[String]) -> bool,
) -> Vec<(String, Vec<String>)> {
    let own = |paths: Paths| {
        paths
            .0
            .into_iter()
            .map(|p| (record.id.clone(), p))
            .collect()
    };
    match &record.item {
        syn::Item::Impl(i) => {
            let syn::Type::Path(ty) = &*i.self_ty else {
                return Vec::new();
            };
            let self_path: Vec<String> = ty
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if !exported_type(&self_path) {
                return Vec::new();
            }
            let type_name = format!("{}::{}", record.module, self_path.join("::"));
            let mut out = Vec::new();
            if let Some((_, trait_path, _)) = &i.trait_ {
                // The trait, its generic arguments and each associated type
                // are nameable as `<Type as Trait>::Item`; the methods'
                // signatures are the trait's own and are read on the trait.
                let mut found = Paths::default();
                found.visit_path(trait_path);
                for item in &i.items {
                    if let syn::ImplItem::Type(t) = item {
                        found.visit_type(&t.ty);
                    }
                }
                let trait_name = trait_path
                    .segments
                    .last()
                    .map_or(String::new(), |s| s.ident.to_string());
                let name = format!("{type_name} as {trait_name}");
                out.extend(found.0.into_iter().map(|p| (name.clone(), p)));
                return out;
            }
            for item in &i.items {
                if let syn::ImplItem::Fn(f) = item {
                    if matches!(f.vis, syn::Visibility::Public(_)) {
                        let mut found = Paths::default();
                        found.visit_signature(&f.sig);
                        let name = format!("{type_name}::{}", f.sig.ident);
                        out.extend(found.0.into_iter().map(|p| (name.clone(), p)));
                    }
                }
            }
            out
        }
        _ if !record.exported => Vec::new(),
        syn::Item::Struct(s) => {
            let mut found = Paths::default();
            for field in s
                .fields
                .iter()
                .filter(|f| matches!(f.vis, syn::Visibility::Public(_)))
            {
                found.visit_type(&field.ty);
            }
            own(found)
        }
        syn::Item::Use(u) => {
            let mut imports = Vec::new();
            crate::source_index::use_tree(&u.tree, &[], &mut imports);
            imports
                .into_iter()
                .map(|i| {
                    let name = i.name.unwrap_or_else(|| String::from("*"));
                    (format!("{}::{name}", record.module), i.path)
                })
                .collect()
        }
        syn::Item::Mod(_) | syn::Item::Macro(_) => Vec::new(),
        item => {
            let mut found = Paths::default();
            found.visit_item(item);
            own(found)
        }
    }
}

/// Every path a declaration writes, bodies and attributes skipped.
#[derive(Default)]
struct Paths(Vec<Vec<String>>);

impl<'ast> Visit<'ast> for Paths {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        self.0
            .push(p.segments.iter().map(|s| s.ident.to_string()).collect());
        syn::visit::visit_path(self, p);
    }
    fn visit_block(&mut self, _: &'ast syn::Block) {}
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
    fn visit_expr(&mut self, _: &'ast syn::Expr) {}
}

//! Definition identities, imports and references from Cargo's Rust targets.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use syn::visit::Visit;

use crate::workspace::{Member, Workspace};

/// One item in its defining module, independent of its exported aliases.
pub struct Record {
    pub id: String,
    pub package: String,
    pub module: String,
    pub file: String,
    pub test: bool,
    pub exported: bool,
    pub item: syn::Item,
    pub references: Vec<Vec<String>>,
    pub imports: Vec<Import>,
}

impl Record {
    pub(crate) fn location(&self) -> String {
        use syn::spanned::Spanned;
        format!("{}:{}", self.file, self.item.span().start().line)
    }
}

/// A local binding introduced by a Rust use declaration.
#[derive(Clone)]
pub struct Import {
    pub name: Option<String>,
    pub path: Vec<String>,
}

/// A `resolve` memo key: a record's address (stable once `records` stops
/// growing) paired with the path it was asked to resolve.
type ResolveCache = RefCell<BTreeMap<(usize, Vec<String>), Option<String>>>;

/// The source inventory for one workspace read.
#[derive(Default)]
pub struct Index {
    pub records: Vec<Record>,
    scopes: BTreeMap<String, Vec<Import>>,
    dependencies: BTreeMap<String, BTreeMap<String, String>>,
    definitions: BTreeSet<String>,
    resolved: BTreeMap<(String, String), BTreeSet<String>>,
    /// Memoized `resolve` results, keyed by a record's address (stable once
    /// `records` stops growing) and its queried path. Several rules walk
    /// overlapping subsets of the same index and re-resolve the same
    /// reference; `resolve` is a pure function of its inputs, so this is safe.
    resolve_cache: ResolveCache,
}

impl Index {
    pub fn read(ws: &Workspace) -> Result<Self> {
        Self::read_members(ws, ws.members(), true)
    }

    pub(crate) fn read_members<'a>(
        ws: &Workspace,
        members: impl Iterator<Item = &'a Member>,
        exports: bool,
    ) -> Result<Self> {
        let mut index = Self::default();
        for member in members {
            index.dependencies.insert(
                member.name.clone(),
                member
                    .dependencies
                    .iter()
                    .map(|d| (d.alias.replace('-', "_"), d.name.replace('-', "_")))
                    .collect(),
            );
            let mut visited = BTreeSet::new();
            for target in &member.targets {
                let path = &target.path;
                let test = target.test_only();
                let root = member.name.replace('-', "_");
                let module = if test {
                    format!(
                        "{root}::__test_{}",
                        path.file_stem().unwrap_or_default().to_string_lossy()
                    )
                } else {
                    root
                };
                index.file(ws, &member.name, path, &module, test, true, &mut visited)?;
            }
        }
        if !exports {
            return Ok(index);
        }
        index.mark_reexports();
        index.resolved = index
            .records
            .iter()
            .map(|r| {
                let mut paths: BTreeSet<String> = r
                    .references
                    .iter()
                    .filter_map(|p| index.resolve(r, p))
                    .collect();
                if let syn::Item::Use(u) = &r.item {
                    let mut imports = Vec::new();
                    use_tree(&u.tree, &[], &mut imports);
                    for import in imports.iter().filter(|i| i.name.is_none()) {
                        if let Some(module) = index.resolve(r, &import.path) {
                            paths.extend(
                                index
                                    .records
                                    .iter()
                                    .filter(|item| item.module == module && item.exported)
                                    .map(|item| item.id.clone()),
                            );
                        }
                    }
                }
                ((r.file.clone(), r.id.clone()), paths)
            })
            .collect();
        Ok(index)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "module traversal carries Cargo target context"
    )]
    fn file(
        &mut self,
        ws: &Workspace,
        package: &str,
        path: &Path,
        module: &str,
        test: bool,
        visible: bool,
        visited: &mut BTreeSet<(PathBuf, String)>,
    ) -> Result<()> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("resolving {}", path.display()))?;
        let path = canonical.as_path();
        if !visited.insert((canonical.clone(), module.to_string())) {
            return Ok(());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let parsed =
            syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))?;
        let dir = if matches!(
            path.file_name().and_then(|p| p.to_str()),
            Some("lib.rs" | "main.rs" | "mod.rs")
        ) {
            path.parent().unwrap_or(path).to_path_buf()
        } else {
            path.with_extension("")
        };
        // Integration target roots resolve sibling modules beside the target, not under its stem.
        let is_target = ws
            .source_members()
            .flat_map(|m| &m.targets)
            .any(|t| t.path == path);
        let dir = if is_target {
            path.parent().unwrap_or(path).to_path_buf()
        } else {
            dir
        };
        self.items(
            ws,
            package,
            &parsed.items,
            path,
            &dir,
            module,
            test || test_only(&parsed.attrs),
            visible,
            visited,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "inline modules retain the defining file and their own child directory"
    )]
    fn items(
        &mut self,
        ws: &Workspace,
        package: &str,
        items: &[syn::Item],
        file: &Path,
        dir: &Path,
        module: &str,
        test: bool,
        visible: bool,
        visited: &mut BTreeSet<(PathBuf, String)>,
    ) -> Result<()> {
        self.scopes.entry(module.to_string()).or_default();
        for (ordinal, item) in items.iter().enumerate() {
            let item_test = test || test_only(attrs(item));
            let public = visibility(item).is_some_and(|v| matches!(v, syn::Visibility::Public(_)));
            if let syn::Item::Macro(m) = item {
                let expanded = expand_contract_macro(m);
                if !expanded.is_empty() {
                    self.items(
                        ws, package, &expanded, file, dir, module, item_test, visible, visited,
                    )?;
                }
            }
            if let syn::Item::Mod(m) = item {
                let child = format!("{module}::{}", m.ident);
                self.definitions.insert(child.clone());
                if let Some((_, items)) = &m.content {
                    self.items(
                        ws,
                        package,
                        items,
                        file,
                        &dir.join(m.ident.to_string()),
                        &child,
                        item_test,
                        visible && public,
                        visited,
                    )?;
                } else {
                    let explicit = m.attrs.iter().find_map(|a| {
                        if !a.path().is_ident("path") {
                            return None;
                        }
                        doc_value(a).map(|p| file.parent().unwrap_or(dir).join(p))
                    });
                    let child_file = explicit.or_else(|| {
                        [
                            dir.join(format!("{}.rs", m.ident)),
                            dir.join(m.ident.to_string()).join("mod.rs"),
                        ]
                        .into_iter()
                        .find(|p| p.exists())
                    });
                    // Platform/feature modules may be generated in OUT_DIR and have no source file.
                    if let Some(child_file) = child_file {
                        self.file(
                            ws,
                            package,
                            &child_file,
                            &child,
                            item_test,
                            visible && public,
                            visited,
                        )?;
                    }
                }
            }
            let name = name(item).unwrap_or_else(|| format!("@{ordinal}"));
            let id = format!("{module}::{name}");
            self.definitions.insert(id.clone());
            if let syn::Item::Trait(t) = item {
                if t.attrs.iter().any(|a| match &a.meta {
                    syn::Meta::List(l) => l.tokens.to_string().contains("automock"),
                    _ => a
                        .path()
                        .segments
                        .last()
                        .is_some_and(|s| s.ident == "automock"),
                }) {
                    self.definitions
                        .insert(format!("{module}::Mock{}", t.ident));
                }
            }
            let mut paths = ReferenceCollector {
                include_tests: item_test,
                ..ReferenceCollector::default()
            };
            paths.visit_item(item);
            if let syn::Item::Use(u) = item {
                let mut imports = Vec::new();
                use_tree(&u.tree, &[], &mut imports);
                self.scopes
                    .entry(module.to_string())
                    .or_default()
                    .extend(imports);
            }
            self.records.push(Record {
                id,
                package: package.to_string(),
                module: module.to_string(),
                file: ws.relative(file),
                test: item_test,
                exported: visible && public && !item_test,
                item: item.clone(),
                references: paths.paths,
                imports: paths.imports,
            });
        }
        Ok(())
    }

    /// Resolve a reference through local imports and cross-crate re-exports.
    pub fn resolve(&self, record: &Record, path: &[String]) -> Option<String> {
        let key = (record as *const Record as usize, path.to_vec());
        if let Some(cached) = self.resolve_cache.borrow().get(&key) {
            return cached.clone();
        }
        let result = self.resolve_inner(
            &record.package,
            &record.module,
            path,
            &record.imports,
            &mut BTreeSet::new(),
        );
        self.resolve_cache.borrow_mut().insert(key, result.clone());
        result
    }

    fn resolve_inner(
        &self,
        package: &str,
        module: &str,
        path: &[String],
        local: &[Import],
        seen: &mut BTreeSet<String>,
    ) -> Option<String> {
        let first = path.first()?;
        if seen.len() >= 64 {
            return None;
        }
        if !seen.insert(format!("{module}/{}", path.join("::"))) {
            return None;
        }
        let root = package.replace('-', "_");
        let mut absolute = match first.as_str() {
            "crate" => format!("{root}::{}", path[1..].join("::")),
            "self" => format!("{module}::{}", path[1..].join("::")),
            "super" => {
                let mut parent = module.to_string();
                let mut offset = 0;
                while path.get(offset).is_some_and(|s| s == "super") {
                    parent = parent
                        .rsplit_once("::")
                        .map_or(parent.clone(), |(p, _)| p.to_string());
                    offset += 1;
                }
                format!("{parent}::{}", path[offset..].join("::"))
            }
            _ => {
                for import in local
                    .iter()
                    .chain(self.scopes.get(module).into_iter().flatten())
                {
                    if import.name.as_ref() == Some(first) {
                        let mut target = import.path.clone();
                        target.extend_from_slice(&path[1..]);
                        if target != path {
                            return self.resolve_inner(package, module, &target, &[], seen);
                        }
                    }
                }
                let candidate = format!("{module}::{}", path.join("::"));
                if self.has_prefix(&candidate) {
                    candidate
                } else if let Some(dep) = self.dependencies.get(package).and_then(|d| d.get(first))
                {
                    format!("{dep}::{}", path[1..].join("::"))
                } else {
                    for import in self
                        .scopes
                        .get(module)
                        .into_iter()
                        .flatten()
                        .filter(|i| i.name.is_none())
                    {
                        let mut target = import.path.clone();
                        target.extend_from_slice(path);
                        if let Some(resolved) =
                            self.resolve_inner(package, module, &target, &[], &mut seen.clone())
                        {
                            if self.has_prefix(&resolved) {
                                return Some(resolved);
                            }
                        }
                    }
                    let candidate = format!("{root}::{}", path.join("::"));
                    if self.has_prefix(&candidate) {
                        candidate
                    } else if matches!(
                        first.as_str(),
                        "std" | "core" | "alloc" | "sqlx" | "axum" | "libc"
                    ) {
                        path.join("::")
                    } else {
                        return None;
                    }
                }
            }
        };
        absolute = absolute.trim_end_matches("::").to_string();
        let parts: Vec<String> = absolute.split("::").map(str::to_string).collect();
        for split in 1..parts.len() {
            let scope = parts[..split].join("::");
            if let Some(imports) = self.scopes.get(&scope) {
                let owner = self
                    .dependencies
                    .keys()
                    .find(|p| p.replace('-', "_") == parts[0]);
                if let Some(owner) = owner {
                    for import in imports {
                        if import.name.as_ref() == Some(&parts[split]) || import.name.is_none() {
                            let mut target = import.path.clone();
                            target.extend_from_slice(
                                &parts[split + usize::from(import.name.is_some())..],
                            );
                            if let Some(resolved) =
                                self.resolve_inner(owner, &scope, &target, &[], &mut seen.clone())
                            {
                                if import.name.is_some() || self.has_prefix(&resolved) {
                                    return Some(resolved);
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(absolute)
    }

    fn has_prefix(&self, path: &str) -> bool {
        if self.definitions.contains(path) || self.scopes.contains_key(path) {
            return true;
        }
        let mut prefix = path;
        loop {
            if self.definitions.contains(prefix) && !self.scopes.contains_key(prefix) {
                return true;
            }
            let Some((parent, _)) = prefix.rsplit_once("::") else {
                return false;
            };
            prefix = parent;
        }
    }

    /// References to an item, including calls to its associated functions.
    pub fn names(&self, record: &Record, id: &str) -> bool {
        self.paths(record)
            .any(|r| r == id || r.starts_with(&format!("{id}::")))
    }

    /// Canonical paths used by this declaration.
    pub fn paths<'a>(&'a self, record: &Record) -> impl Iterator<Item = &'a String> {
        self.resolved
            .get(&(record.file.clone(), record.id.clone()))
            .into_iter()
            .flatten()
    }

    fn mark_reexports(&mut self) {
        let mut exported = BTreeSet::new();
        for r in &self.records {
            if !r.exported {
                continue;
            }
            if let syn::Item::Use(u) = &r.item {
                let mut imports = Vec::new();
                use_tree(&u.tree, &[], &mut imports);
                for i in imports {
                    if let Some(target) = self.resolve(r, &i.path) {
                        if i.name.is_some() {
                            exported.insert(target);
                        } else {
                            for item in &self.records {
                                if item.module == target
                                    && visibility(&item.item)
                                        .is_some_and(|v| matches!(v, syn::Visibility::Public(_)))
                                {
                                    exported.insert(item.id.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        for r in &mut self.records {
            r.exported |= !r.test && exported.contains(&r.id);
        }
    }
}

/// The attributes attached to a declaration.
pub fn attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        _ => &[],
    }
}

fn visibility(item: &syn::Item) -> Option<&syn::Visibility> {
    match item {
        syn::Item::Struct(i) => Some(&i.vis),
        syn::Item::Enum(i) => Some(&i.vis),
        syn::Item::Trait(i) => Some(&i.vis),
        syn::Item::Type(i) => Some(&i.vis),
        syn::Item::Fn(i) => Some(&i.vis),
        syn::Item::Use(i) => Some(&i.vis),
        syn::Item::Mod(i) => Some(&i.vis),
        syn::Item::Const(i) => Some(&i.vis),
        syn::Item::Static(i) => Some(&i.vis),
        _ => None,
    }
}

fn name(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Struct(i) => Some(i.ident.to_string()),
        syn::Item::Enum(i) => Some(i.ident.to_string()),
        syn::Item::Trait(i) => Some(i.ident.to_string()),
        syn::Item::Type(i) => Some(i.ident.to_string()),
        syn::Item::Fn(i) => Some(i.sig.ident.to_string()),
        syn::Item::Mod(i) => Some(i.ident.to_string()),
        syn::Item::Const(i) => Some(i.ident.to_string()),
        syn::Item::Static(i) => Some(i.ident.to_string()),
        syn::Item::Impl(i) => {
            let mut paths = ReferenceCollector::default();
            paths.visit_type(&i.self_ty);
            let ty = paths
                .paths
                .iter()
                .map(|p| p.join("::"))
                .collect::<Vec<_>>()
                .join(" ");
            let trait_ = i.trait_.as_ref().map(|(_, p, _)| {
                p.segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::")
            });
            Some(format!("impl {} for {ty}", trait_.unwrap_or_default()))
        }
        syn::Item::Use(u) => {
            let mut imports = Vec::new();
            use_tree(&u.tree, &[], &mut imports);
            let mut names: Vec<_> = imports
                .iter()
                .map(|i| {
                    format!(
                        "{} as {}",
                        i.path.join("::"),
                        i.name.as_deref().unwrap_or("*")
                    )
                })
                .collect();
            names.sort();
            Some(format!("use {}", names.join(", ")))
        }
        syn::Item::Macro(m) => Some(format!(
            "macro {}",
            m.ident.as_ref().map_or_else(
                || m.mac
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::"),
                ToString::to_string
            )
        )),
        _ => None,
    }
}

/// A string-valued attribute, including doc and path attributes.
pub fn doc_value(attr: &syn::Attribute) -> Option<String> {
    if let syn::Meta::NameValue(n) = &attr.meta {
        if let syn::Expr::Lit(l) = &n.value {
            if let syn::Lit::Str(s) = &l.lit {
                return Some(s.value());
            }
        }
    }
    None
}

/// Whether a cfg expression cannot enable an item in a production build.
pub fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test"))
        || attrs
            .iter()
            .filter(|a| a.path().is_ident("cfg"))
            .any(|a| a.parse_args::<syn::Meta>().is_ok_and(|m| cfg_test(&m)))
}

fn cfg_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(p) => p.is_ident("test"),
        syn::Meta::NameValue(n) if n.path.is_ident("feature") => {
            if let syn::Expr::Lit(l) = &n.value {
                if let syn::Lit::Str(s) = &l.lit {
                    return matches!(s.value().as_str(), "test-support" | "db-tests");
                }
            }
            false
        }
        syn::Meta::List(l) => {
            use syn::parse::Parser;
            let parser = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
            let Ok(children) = parser.parse2(l.tokens.clone()) else {
                return false;
            };
            if l.path.is_ident("any") {
                !children.is_empty() && children.iter().all(cfg_test)
            } else if l.path.is_ident("all") {
                children.iter().any(cfg_test)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Paths written by a Rust item, excluding documentation and nested modules.
#[derive(Default)]
pub struct ReferenceCollector {
    pub paths: Vec<Vec<String>>,
    pub imports: Vec<Import>,
    pub include_tests: bool,
    pub omit_import_paths: bool,
}

impl<'ast> Visit<'ast> for ReferenceCollector {
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if self.include_tests || !test_only(&f.attrs) {
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
    fn visit_path(&mut self, p: &'ast syn::Path) {
        self.paths
            .push(p.segments.iter().map(|s| s.ident.to_string()).collect());
        syn::visit::visit_path(self, p);
    }
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        let mut imports = Vec::new();
        use_tree(&u.tree, &[], &mut imports);
        if !self.omit_import_paths {
            self.paths.extend(imports.iter().map(|i| i.path.clone()));
        }
        self.imports.extend(imports);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        use syn::parse::Parser;
        self.visit_path(&m.path);
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(expressions) = parser.parse2(m.tokens.clone()) {
            for e in &expressions {
                self.visit_expr(e);
            }
        }
    }
}

fn expand_contract_macro(m: &syn::ItemMacro) -> Vec<syn::Item> {
    use syn::parse::Parser;
    let Some(last) = m.mac.path.segments.last() else {
        return Vec::new();
    };
    let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<Vec<syn::Item>> {
        let attrs = input.call(syn::Attribute::parse_outer)?;
        let _: syn::Visibility = input.parse()?;
        let ident: syn::Ident = input.parse()?;
        if input.peek(syn::token::Bracket) {
            let extras;
            syn::bracketed!(extras in input);
            let _: proc_macro2::TokenStream = extras.parse()?;
        }
        let body;
        syn::braced!(body in input);
        let _: proc_macro2::TokenStream = body.parse()?;
        if last.ident == "string_enum" {
            return Ok(vec![syn::parse_quote!(#(#attrs)* pub enum #ident {})]);
        }
        let mock = syn::Ident::new(&format!("Mock{ident}"), ident.span());
        let mut items = vec![syn::parse_quote!(pub struct #mock;)];
        while !input.is_empty() {
            let _attrs = input.call(syn::Attribute::parse_outer)?;
            let _: syn::Token![impl] = input.parse()?;
            let trait_path: syn::Path = input.parse()?;
            let _: syn::Token![for] = input.parse()?;
            let _: syn::Type = input.parse()?;
            let methods;
            syn::braced!(methods in input);
            let _: proc_macro2::TokenStream = methods.parse()?;
            items.push(syn::parse_quote!(impl #trait_path for #mock {}));
        }
        Ok(items)
    };
    if matches!(last.ident.to_string().as_str(), "string_enum" | "mock") {
        parser.parse2(m.mac.tokens.clone()).unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Flatten grouped imports without losing aliases or globs.
pub fn use_tree(tree: &syn::UseTree, prefix: &[String], out: &mut Vec<Import>) {
    match tree {
        syn::UseTree::Path(p) => {
            let mut path = prefix.to_vec();
            path.push(p.ident.to_string());
            use_tree(&p.tree, &path, out);
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                use_tree(t, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => out.push(Import {
            name: None,
            path: prefix.to_vec(),
        }),
        syn::UseTree::Name(n) => {
            let mut path = prefix.to_vec();
            if n.ident != "self" {
                path.push(n.ident.to_string());
            }
            out.push(Import {
                name: path.last().cloned(),
                path,
            });
        }
        syn::UseTree::Rename(r) => {
            let mut path = prefix.to_vec();
            if r.ident != "self" {
                path.push(r.ident.to_string());
            }
            out.push(Import {
                name: Some(r.rename.to_string()),
                path,
            });
        }
    }
}

/// The canonical trait implemented by an impl declaration.
pub fn implemented_trait(index: &Index, r: &Record) -> Option<String> {
    let syn::Item::Impl(i) = &r.item else {
        return None;
    };
    let (_, path, _) = i.trait_.as_ref()?;
    index.resolve(
        r,
        &path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture records must exist")]
mod tests {
    use crate::fixtures::Fixture;
    #[test]
    fn aliases_and_reexport_chains_resolve_to_the_definition() {
        let mut f = Fixture::new();
        f.member(
            "origin",
            "",
            "mod private { pub struct Data; } pub use private::Data as Value;",
        );
        f.member(
            "bridge",
            "renamed = { package = 'origin', path = '../origin' }",
            "pub use renamed::Value as Shared;",
        );
        f.member(
            "reader",
            "bridge = { path = '../bridge' }",
            "use bridge::Shared as Local; pub fn read(_: Local) {}",
        );
        let ws = f.load();
        let index = ws.index().expect("index");
        let read = index
            .records
            .iter()
            .find(|r| r.id == "reader::read")
            .expect("read");
        assert!(index.names(read, "origin::private::Data"));
        assert!(
            index
                .records
                .iter()
                .find(|r| r.id == "origin::private::Data")
                .expect("data")
                .exported
        );
    }
    #[test]
    fn globs_do_not_capture_an_external_crate_as_a_local_module() {
        let mut f = Fixture::new();
        f.member("floor", "", "pub struct Value;");
        f.member(
            "reader",
            "floor = { path = '../floor' }",
            "mod inner { use floor::*; pub fn read(_: Value) {} pub fn other(_: floor::Value) {} }",
        );
        let ws = f.load();
        let index = ws.index().expect("index");
        for name in ["reader::inner::read", "reader::inner::other"] {
            let r = index.records.iter().find(|r| r.id == name).expect("reader");
            assert!(index.names(r, "floor::Value"), "{name}");
        }
    }
    #[test]
    fn private_orphan_and_test_sources_are_distinguished() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod private; #[cfg(any(test, feature = \"test-support\"))] mod tests; #[cfg(not(test))] pub struct Production;");
        f.write("sample/src/private.rs", "pub struct Hidden;");
        f.write("sample/src/tests.rs", "pub struct Fake;");
        f.write("sample/src/orphan.rs", "pub struct Orphan;");
        let ws = f.load();
        let index = ws.index().expect("index");
        assert!(!index.records.iter().any(|r| r.id.ends_with("::Orphan")));
        assert!(
            index
                .records
                .iter()
                .any(|r| r.id.ends_with("::Fake") && r.test)
        );
        assert!(
            index
                .records
                .iter()
                .any(|r| r.id.ends_with("::Hidden") && !r.exported && !r.test)
        );
        assert!(
            index
                .records
                .iter()
                .any(|r| r.id.ends_with("::Production") && r.exported)
        );
    }
}

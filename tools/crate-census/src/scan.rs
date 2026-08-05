//! Pass-1 collection: a syn visitor that builds the per-file module tables and
//! collects raw references, plus a token-stream scanner for the attribute,
//! doc-comment, and macro-invocation reference classes syn hands back opaque.

use std::collections::{HashMap, HashSet};

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens as _;
use syn::spanned::Spanned as _;
use syn::visit::Visit;
use syn::{
    Attribute, Expr, ExprLit, Item, ItemMacro, ItemMod, ItemUse, Lit, Macro, Meta, Path, UseTree,
    Visibility,
};

use crate::bands::ScopeKind;
use crate::model::{ModPath, RefClass, Tree};
use crate::RawRef;

pub struct MacroDef {
    pub file: String,
    pub name: String,
    pub emits_path: bool,
}

struct PendingRef {
    from_mod: ModPath,
    segs: Vec<String>,
    class: RefClass,
    line: usize,
    in_test_cfg: bool,
}

pub struct FileVisitor {
    stack: ModPath,
    test_cfg_depth: u32,
    scope: ScopeKind,
    source_crate: String,

    modules: HashSet<ModPath>,
    children: HashMap<ModPath, HashSet<String>>,
    uses: HashMap<ModPath, HashMap<String, Vec<String>>>,
    glob_uses: HashMap<ModPath, Vec<Vec<String>>>,
    reexports: HashMap<ModPath, HashMap<String, Vec<String>>>,
    glob_reexports: HashMap<ModPath, Vec<Vec<String>>>,
    defined: HashMap<ModPath, HashSet<String>>,
    cfg_test_mods: Vec<ModPath>,
    refs: Vec<PendingRef>,
    macros: Vec<(String, bool)>,
}

impl FileVisitor {
    pub fn new(base_mod: ModPath, scope: ScopeKind, source_crate: String) -> Self {
        let mut modules = HashSet::new();
        modules.insert(base_mod.clone());
        FileVisitor {
            stack: base_mod,
            test_cfg_depth: 0,
            scope,
            source_crate,
            modules,
            children: HashMap::new(),
            uses: HashMap::new(),
            glob_uses: HashMap::new(),
            reexports: HashMap::new(),
            glob_reexports: HashMap::new(),
            defined: HashMap::new(),
            cfg_test_mods: Vec::new(),
            refs: Vec::new(),
            macros: Vec::new(),
        }
    }

    fn define(&mut self, name: &syn::Ident) {
        self.defined.entry(self.stack.clone()).or_default().insert(name.to_string());
    }

    fn push_ref(&mut self, segs: Vec<String>, class: RefClass, line: usize) {
        if segs.len() < 2 {
            return;
        }
        self.refs.push(PendingRef {
            from_mod: self.stack.clone(),
            segs,
            class,
            line,
            in_test_cfg: self.test_cfg_depth > 0,
        });
    }

    pub fn commit(
        self,
        tree: &mut Tree,
        raw: &mut Vec<RawRef>,
        macro_defs: &mut Vec<MacroDef>,
        rel: &str,
    ) {
        // Merge tables. Each module path is owned by exactly one file, so these
        // are inserts/unions with no real conflict.
        for m in self.modules {
            insert_with_ancestors(&mut tree.modules, m);
        }
        for (k, v) in self.children {
            tree.children.entry(k).or_default().extend(v);
        }
        for (k, v) in self.uses {
            tree.uses.entry(k).or_default().extend(v);
        }
        for (k, v) in self.glob_uses {
            tree.glob_uses.entry(k).or_default().extend(v);
        }
        for (k, v) in self.reexports {
            tree.reexports.entry(k).or_default().extend(v);
        }
        for (k, v) in self.glob_reexports {
            tree.glob_reexports.entry(k).or_default().extend(v);
        }
        for (k, v) in self.defined {
            tree.defined.entry(k).or_default().extend(v);
        }
        for p in self.cfg_test_mods {
            tree.cfg_test_mods.insert(p);
        }
        for r in self.refs {
            raw.push(RawRef {
                scope: self.scope,
                source_crate: self.source_crate.clone(),
                from_mod: r.from_mod,
                segs: r.segs,
                class: r.class,
                file: rel.to_string(),
                line: r.line,
                in_test_cfg: r.in_test_cfg,
            });
        }
        for (name, emits_path) in self.macros {
            macro_defs.push(MacroDef { file: rel.to_string(), name, emits_path });
        }
    }
}

fn insert_with_ancestors(set: &mut HashSet<ModPath>, m: ModPath) {
    let mut cur = m;
    loop {
        set.insert(cur.clone());
        if cur.pop().is_none() || cur.is_empty() {
            set.insert(Vec::new());
            break;
        }
    }
}

impl<'ast> Visit<'ast> for FileVisitor {
    fn visit_item(&mut self, i: &'ast Item) {
        match i {
            Item::Struct(s) => self.define(&s.ident),
            Item::Enum(e) => self.define(&e.ident),
            Item::Fn(f) => self.define(&f.sig.ident),
            Item::Trait(t) => self.define(&t.ident),
            Item::TraitAlias(t) => self.define(&t.ident),
            Item::Type(t) => self.define(&t.ident),
            Item::Const(c) => self.define(&c.ident),
            Item::Static(s) => self.define(&s.ident),
            Item::Union(u) => self.define(&u.ident),
            Item::Macro(m) => {
                if let Some(id) = &m.ident {
                    self.define(id);
                }
            }
            _ => {}
        }
        syn::visit::visit_item(self, i);
    }

    fn visit_item_mod(&mut self, m: &'ast ItemMod) {
        let name = m.ident.to_string();
        self.children.entry(self.stack.clone()).or_default().insert(name.clone());
        let is_test = attrs_have_cfg_test(&m.attrs);
        if is_test {
            // Record the full module path so a reference in this module — or, for a
            // file-module (`#[cfg(test)] mod tests;`) whose body `discover()` parses
            // as its own file, in that file — is scored as test-context, not a
            // production edge.
            let mut path = self.stack.clone();
            path.push(name.clone());
            self.cfg_test_mods.push(path);
        }
        if let Some((_brace, items)) = &m.content {
            self.stack.push(name);
            self.modules.insert(self.stack.clone());
            if is_test {
                self.test_cfg_depth += 1;
            }
            for a in &m.attrs {
                self.visit_attribute(a);
            }
            for it in items {
                self.visit_item(it);
            }
            if is_test {
                self.test_cfg_depth -= 1;
            }
            self.stack.pop();
        }
    }

    fn visit_item_use(&mut self, u: &'ast ItemUse) {
        let is_reexport = !matches!(u.vis, Visibility::Inherited);
        let mut leaves: Vec<(String, Vec<String>, bool, usize)> = Vec::new();
        flatten_use_tree(&u.tree, &mut Vec::new(), &mut leaves);
        for (alias, segs, is_glob, line) in leaves {
            if is_glob {
                self.glob_uses.entry(self.stack.clone()).or_default().push(segs.clone());
                if is_reexport {
                    self.glob_reexports.entry(self.stack.clone()).or_default().push(segs.clone());
                }
                self.push_ref(segs, RefClass::Code, line);
            } else {
                self.uses
                    .entry(self.stack.clone())
                    .or_default()
                    .entry(alias.clone())
                    .or_insert_with(|| segs.clone());
                if is_reexport {
                    self.reexports
                        .entry(self.stack.clone())
                        .or_default()
                        .insert(alias, segs.clone());
                }
                self.push_ref(segs, RefClass::Code, line);
            }
        }
    }

    fn visit_item_macro(&mut self, im: &'ast ItemMacro) {
        if im.mac.path.is_ident("macro_rules") {
            let name = im.ident.as_ref().map(ToString::to_string).unwrap_or_default();
            let emits = tokens_have_cross_module_path(&im.mac.tokens);
            self.macros.push((name, emits));
            return; // a macro_rules body is a template, not resolved references
        }
        syn::visit::visit_item_macro(self, im);
    }

    fn visit_macro(&mut self, m: &'ast Macro) {
        for (segs, line) in scan_tokens(&m.tokens) {
            self.push_ref(segs, RefClass::Macro, line);
        }
        // visits m.path (the macro name) as a Path
        syn::visit::visit_macro(self, m);
    }

    fn visit_path(&mut self, p: &'ast Path) {
        if p.segments.len() >= 2 {
            let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
            let line = p.segments[0].ident.span().start().line;
            self.push_ref(segs, RefClass::Code, line);
        }
        // Recurse into generic arguments to catch nested paths (`HashMap<_, crate::X>`).
        syn::visit::visit_path(self, p);
    }

    fn visit_attribute(&mut self, a: &'ast Attribute) {
        if a.path().is_ident("doc") {
            if let Meta::NameValue(nv) = &a.meta {
                if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = &nv.value {
                    let line = s.span().start().line;
                    for segs in scan_doc_string(&s.value()) {
                        self.push_ref(segs, RefClass::Doc, line);
                    }
                }
            }
            return;
        }
        let tokens = a.meta.to_token_stream();
        for (segs, line) in scan_tokens(&tokens) {
            self.push_ref(segs, RefClass::Attribute, line);
        }
    }
}

/// Flatten a `use` tree into `(exported-name, full-path-segments, is_glob, line)`.
fn flatten_use_tree(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    out: &mut Vec<(String, Vec<String>, bool, usize)>,
) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use_tree(&p.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let mut segs = prefix.clone();
            segs.push(n.ident.to_string());
            out.push((n.ident.to_string(), segs, false, n.ident.span().start().line));
        }
        UseTree::Rename(r) => {
            let mut segs = prefix.clone();
            segs.push(r.ident.to_string());
            out.push((r.rename.to_string(), segs, false, r.ident.span().start().line));
        }
        UseTree::Glob(g) => {
            out.push((String::new(), prefix.clone(), true, g.star_token.span().start().line));
        }
        UseTree::Group(grp) => {
            for item in &grp.items {
                flatten_use_tree(item, prefix, out);
            }
        }
    }
}

fn attrs_have_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("test")
    })
}

// -- token-stream path extraction ------------------------------------------

/// Extract `Ident (:: Ident)+` sequences (len >= 2) and anchored paths inside
/// string literals from a token stream, recursing into delimited groups.
pub fn scan_tokens(ts: &TokenStream) -> Vec<(Vec<String>, usize)> {
    let mut out = Vec::new();
    scan_ts(ts, &mut out);
    out
}

fn scan_ts(ts: &TokenStream, out: &mut Vec<(Vec<String>, usize)>) {
    let toks: Vec<TokenTree> = ts.clone().into_iter().collect();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            TokenTree::Group(g) => {
                scan_ts(&g.stream(), out);
                i += 1;
            }
            TokenTree::Ident(id) => {
                let mut segs = vec![id.to_string()];
                let line = id.span().start().line;
                let mut j = i + 1;
                while let (Some(TokenTree::Punct(p1)), Some(TokenTree::Punct(p2)), Some(TokenTree::Ident(id2))) =
                    (toks.get(j), toks.get(j + 1), toks.get(j + 2))
                {
                    if p1.as_char() == ':' && p2.as_char() == ':' {
                        segs.push(id2.to_string());
                        j += 3;
                    } else {
                        break;
                    }
                }
                if segs.len() >= 2 {
                    out.push((segs, line));
                }
                i = j.max(i + 1);
            }
            TokenTree::Literal(lit) => {
                let s = lit.to_string();
                if s.starts_with('"') {
                    let line = lit.span().start().line;
                    for segs in scan_doc_string(&s) {
                        out.push((segs, line));
                    }
                }
                i += 1;
            }
            TokenTree::Punct(_) => i += 1,
        }
    }
}

fn tokens_have_cross_module_path(ts: &TokenStream) -> bool {
    scan_tokens(ts).iter().any(|(segs, _)| {
        matches!(segs.first().map(String::as_str), Some("crate" | "super" | "self" | "$crate"))
            || segs.first().is_some_and(|h| h.starts_with("moatless_"))
    })
}

/// Extract anchored path forms (`crate::`, `super::`, `self::`, `moatless_*::`)
/// from a doc string or string literal.
pub fn scan_doc_string(s: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // word boundary?
        let boundary = i == 0 || !is_ident_char(chars[i - 1]);
        if boundary && is_ident_start(chars[i]) {
            let (ident, next) = read_ident(&chars, i);
            let anchor = matches!(ident.as_str(), "crate" | "super" | "self")
                || ident.starts_with("moatless_");
            if anchor && chars.get(next) == Some(&':') && chars.get(next + 1) == Some(&':') {
                let mut segs = vec![ident];
                let mut k = next;
                while chars.get(k) == Some(&':') && chars.get(k + 1) == Some(&':') {
                    let start = k + 2;
                    if start >= chars.len() || !is_ident_start(chars[start]) {
                        break;
                    }
                    let (seg, after) = read_ident(&chars, start);
                    segs.push(seg);
                    k = after;
                }
                if segs.len() >= 2 {
                    out.push(segs);
                }
                i = k;
                continue;
            }
            i = next;
            continue;
        }
        i += 1;
    }
    out
}

fn read_ident(chars: &[char], start: usize) -> (String, usize) {
    let mut end = start;
    while end < chars.len() && is_ident_char(chars[end]) {
        end += 1;
    }
    (chars[start..end].iter().collect(), end)
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

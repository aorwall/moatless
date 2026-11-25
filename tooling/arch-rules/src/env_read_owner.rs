//! Each binary reads its environment once, in its `Config`, and passes values in.

use crate::composition_root;
use crate::source_index::Record;
use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;

pub const RULE: &str = "env-read-owner";

fn reads_env(path: &str) -> bool {
    matches!(
        path,
        "std::env::var" | "std::env::var_os" | "std::env::vars" | "std::env::vars_os"
    )
}

/// A crate's top-level `config` module, its backend composition root, or a
/// binary's entry file.
fn owner(file: &str, entries: &BTreeSet<String>) -> bool {
    entries.contains(file)
        || file.ends_with("/src/config.rs")
        || file.contains("/src/config/")
        || composition_root::root(file)
}

/// One call: the fn it sits in, the path it called, and its literal first
/// argument, if any. A bare path reference is a call with no argument.
struct Call {
    within: Option<String>,
    path: Vec<String>,
    name: Option<String>,
}

#[derive(Default)]
struct Calls {
    within: Option<String>,
    found: Vec<Call>,
}
impl Calls {
    fn push(&mut self, path: &syn::Path, name: Option<String>) {
        self.found.push(Call {
            within: self.within.clone(),
            path: path.segments.iter().map(|s| s.ident.to_string()).collect(),
            name,
        });
    }
    fn within(&mut self, name: String, visit: impl FnOnce(&mut Self)) {
        let outer = self.within.replace(name);
        visit(self);
        self.within = outer;
    }
}
impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        let syn::Expr::Path(p) = &*c.func else {
            return syn::visit::visit_expr_call(self, c);
        };
        let name = match c.args.first() {
            Some(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            })) => Some(s.value()),
            _ => None,
        };
        self.push(&p.path, name);
        for arg in &c.args {
            self.visit_expr(arg);
        }
    }
    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        self.push(&p.path, None);
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.within(f.sig.ident.to_string(), |v| syn::visit::visit_item_fn(v, f));
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.within(f.sig.ident.to_string(), |v| {
            syn::visit::visit_impl_item_fn(v, f);
        });
    }
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
}

/// Where one crate module reads the environment, and where it reads each name.
/// `locations` holds only the reads no `arch-allow` excuses; `names` holds all.
#[derive(Default)]
struct Site {
    owned: bool,
    locations: BTreeSet<String>,
    names: BTreeMap<String, BTreeSet<String>>,
}

/// Judge the gathered reads: `sites` keyed by (crate, module), and `binaries`
/// as the crates each executable links. Two binaries may each read a name in
/// their own `Config`; one binary reads it in one crate.
fn judge(
    sites: &BTreeMap<(String, String), Site>,
    binaries: &[BTreeSet<String>],
) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut readers: BTreeMap<&str, BTreeMap<&str, BTreeSet<&String>>> = BTreeMap::new();
    for ((package, module), site) in sites {
        for (name, at) in &site.names {
            readers
                .entry(name)
                .or_default()
                .entry(package)
                .or_default()
                .extend(at);
        }
        if site.owned || site.locations.is_empty() {
            continue;
        }
        let names: Vec<_> = site
            .names
            .iter()
            .filter(|(_, at)| !at.is_disjoint(&site.locations))
            .map(|(name, _)| name.as_str())
            .collect();
        let names = if names.is_empty() {
            String::new()
        } else {
            format!(" ({})", names.join(", "))
        };
        out.push(Violation {
            rule: RULE,
            key: format!("{module} reads the environment"),
            message: format!(
                "{module} reads the environment{names}; read it in the binary's Config and pass the value in"
            ),
            locations: site.locations.iter().cloned().collect(),
        });
    }
    for (name, packages) in readers {
        let shared: BTreeSet<&str> = binaries
            .iter()
            .map(|b| {
                packages
                    .keys()
                    .copied()
                    .filter(|p| b.contains(*p))
                    .collect::<Vec<_>>()
            })
            .filter(|p| p.len() > 1)
            .flatten()
            .collect();
        if shared.is_empty() {
            continue;
        }
        let locations = shared
            .iter()
            .flat_map(|p| packages[p].iter().map(|l| (*l).clone()))
            .collect();
        let packages = shared.into_iter().collect::<Vec<_>>().join(", ");
        out.push(Violation {
            rule: RULE,
            key: format!("{name} is read in {packages}"),
            message: format!(
                "{name} is read in {packages}, which one binary links; read it in that binary's Config"
            ),
            locations,
        });
    }
    out
}

/// Fns in records `keep` accepts that read the environment, by resolved path,
/// closed over the kept fns that call them. With `pass_through`, only fns that
/// hand a reader a non-literal name, so a literal passed to one is the name read.
fn readers(
    calls: &[(&Record, Call, String)],
    pass_through: bool,
    keep: impl Fn(&Record) -> bool,
) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    loop {
        let before = found.len();
        for (r, call, path) in calls {
            let Some(within) = &call.within else { continue };
            if keep(r)
                && (reads_env(path) || found.contains(path))
                && !(pass_through && call.name.is_some())
            {
                found.insert(format!("{}::{within}", r.module));
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
    let entries: BTreeSet<_> = ws
        .members()
        .flat_map(|m| &m.targets)
        .filter(|t| t.executable())
        .map(|t| ws.relative(&t.path))
        .collect();
    let mut calls = Vec::new();
    for r in index.records.iter().filter(|r| !r.test) {
        if matches!(r.item, syn::Item::Use(_)) {
            continue;
        }
        let mut found = Calls::default();
        found.visit_item(&r.item);
        for call in found.found {
            let path = index
                .resolve(r, &call.path)
                .unwrap_or_else(|| call.path.join("::"));
            calls.push((r, call, path));
        }
    }
    let helpers = readers(&calls, true, |_| true);
    // A call to an owner's reader reads the environment where it runs, so a
    // `Config` helper called at runtime from outside an owner is a breach too.
    // Deliberately owners only: a reader outside one is already its own breach.
    let readers = readers(&calls, false, |r| owner(&r.file, &entries));
    let mut sites: BTreeMap<(String, String), Site> = BTreeMap::new();
    for (r, call, path) in &calls {
        if !(reads_env(path) || readers.contains(path)) {
            continue;
        }
        let site = sites
            .entry((r.package.clone(), r.module.clone()))
            .or_default();
        site.owned |= owner(&r.file, &entries);
        if !annotations::permits(r, RULE) {
            site.locations.insert(r.location());
        }
        if let Some(name) = call
            .name
            .as_ref()
            .filter(|_| reads_env(path) || helpers.contains(path))
        {
            site.names
                .entry(name.clone())
                .or_default()
                .insert(r.location());
        }
    }
    let binaries: Vec<BTreeSet<String>> = ws
        .members()
        .filter(|m| m.targets.iter().any(crate::workspace::Target::executable))
        .map(|bin| {
            ws.members()
                .map(|m| m.name.clone())
                .filter(|m| *m == bin.name || ws.reaches(&bin.name, m))
                .collect()
        })
        .collect();
    Ok(judge(&sites, &binaries))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;

    fn keys(found: &[Violation]) -> Vec<&str> {
        found.iter().map(|v| v.key.as_str()).collect()
    }

    #[test]
    fn config_modules_and_binary_entries_own_reads() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod config; mod deep; mod tests_only;");
        f.write(
            "sample/src/config.rs",
            "pub fn home() -> Option<String> { std::env::var(\"HOME\").ok() }",
        );
        f.write(
            "sample/src/deep.rs",
            "use std::env; pub fn port() -> Option<String> { env::var(\"PORT\").ok() }",
        );
        f.write(
            "sample/src/tests_only.rs",
            "#[cfg(test)] mod t { fn f() { std::env::var(\"X\").ok(); } }",
        );
        f.write("sample/Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'\n[[bin]]\nname='entry'\npath='src/main.rs'\n");
        f.write(
            "sample/src/main.rs",
            "fn main() { let _ = std::env::var_os(\"MAIN_ONLY\"); }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(
            keys(&found),
            ["sample::deep reads the environment"],
            "{found:?}"
        );
    }

    #[test]
    fn a_name_read_in_two_crates_of_one_binary_names_both() {
        let mut f = Fixture::new();
        f.member("alpha", "", "mod config;");
        f.write(
            "alpha/src/config.rs",
            "fn opt(k: &str) -> Option<String> { std::env::var(k).ok() } \
             fn or(k: &str) -> String { opt(k).unwrap_or_default() } \
             pub fn home() { or(\"HOME\"); }",
        );
        f.member("beta", "", "mod config;");
        f.write(
            "beta/src/config.rs",
            "use std::env::var as read; pub fn home() { read(\"HOME\").ok(); read(\"BETA\").ok(); }",
        );
        f.member("lone", "", "mod config;");
        f.write(
            "lone/src/config.rs",
            "pub fn home() { std::env::var(\"HOME\").ok(); }",
        );
        f.member(
            "app",
            "alpha = { path = '../alpha' }\nbeta = { path = '../beta' }",
            "",
        );
        f.write("app/src/main.rs", "fn main() {}");
        f.member("other", "lone = { path = '../lone' }", "");
        f.write("other/src/main.rs", "fn main() {}");
        let found = check(&f.load()).expect("check");
        assert_eq!(keys(&found), ["HOME is read in alpha, beta"], "{found:?}");
    }

    #[test]
    fn an_annotated_item_is_excused_and_a_nested_config_is_not_an_owner() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod agent;");
        f.write(
            "sample/src/agent/mod.rs",
            "mod config; /// arch-allow(env-read-owner): the child process inherits it\n\
             pub fn f() { std::env::vars().count(); } \
             pub fn g() { std::env::var(\"G\").ok(); }",
        );
        f.write(
            "sample/src/agent/config.rs",
            "pub fn f() { std::env::var(\"X\").ok(); }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(
            keys(&found),
            [
                "sample::agent reads the environment",
                "sample::agent::config reads the environment"
            ],
            "{found:?}"
        );
        assert!(found[0].message.contains("(G)"), "{found:?}");
        assert_eq!(found[0].locations.len(), 1, "{found:?}");
    }

    #[test]
    fn a_helper_called_through_its_module_path_records_the_name() {
        let mut f = Fixture::new();
        f.member("alpha", "", "mod config; mod server;");
        f.write(
            "alpha/src/config.rs",
            "pub fn env_or(k: &str) -> String { std::env::var(k).unwrap_or_default() }",
        );
        f.write(
            "alpha/src/server.rs",
            "pub fn port() { crate::config::env_or(\"PORT\"); }",
        );
        f.member("beta", "", "mod config;");
        f.write(
            "beta/src/config.rs",
            "pub fn port() { std::env::var(\"PORT\").ok(); }",
        );
        f.member(
            "app",
            "alpha = { path = '../alpha' }\nbeta = { path = '../beta' }",
            "",
        );
        f.write("app/src/main.rs", "fn main() {}");
        let found = check(&f.load()).expect("check");
        assert_eq!(
            keys(&found),
            [
                "alpha::server reads the environment",
                "PORT is read in alpha, beta"
            ],
            "{found:?}"
        );
    }

    #[test]
    fn a_config_helper_called_at_runtime_from_another_crate_is_a_read() {
        let mut f = Fixture::new();
        f.member("alpha", "", "pub mod config;");
        f.write(
            "alpha/src/config.rs",
            "pub fn api_base() -> String { std::env::var(\"API\").unwrap_or_default() } \
             pub fn load() -> String { api_base() }",
        );
        f.member(
            "beta",
            "alpha = { path = '../alpha' }",
            "mod app; mod client;",
        );
        f.write(
            "beta/src/client.rs",
            "pub fn get() -> String { alpha::config::api_base() }",
        );
        f.write(
            "beta/src/app.rs",
            "pub fn run() -> String { crate::client::get() }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(
            keys(&found),
            ["beta::client reads the environment"],
            "{found:?}"
        );
    }
}

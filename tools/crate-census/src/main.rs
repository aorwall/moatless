//! Cross-crate reference census for the ports-and-crates program.
//!
//! Resolves every path form in the tree — `crate::`, `super::`, `self::`,
//! `use`-imported names, and `pub use` re-export chains — to the crate that
//! *defines* the symbol, classifies each reference against the target-crate
//! band order, and reports the illegal edges that must go to zero before a
//! carve. It is the ratchet for Step 10: run it before each carve and again
//! after to prove an edge reached zero.
//!
//! It has a defined death: once Step 10i lands, every crate is real and Cargo
//! enforces the bands — an illegal edge stops compiling and this census is
//! redundant. Delete this tool at 10i rather than keeping it as scaffolding.
//!
//! Known limitation: syn does not expand macros, so a cross-module path
//! *generated inside a macro body* (not present in the macro input) is
//! invisible. The tool prints its `macro_rules!` inventory over the whole
//! census scope on every run so a newly added path-emitting macro surfaces as
//! a diff; today every such macro is a local SQL-string builder that emits no
//! path (see the inventory).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use syn::visit::Visit;

mod bands;
mod model;
mod resolve;
mod scan;

use bands::{crate_of_backend_mod, external_crate, floor_crate_of_dir, CrateAssignment, ScopeKind, CENSUS_ROOTS};
use model::{ModPath, RefClass, RefRecord, Resolution, Tree};
use resolve::resolve_backend;
use scan::{FileVisitor, MacroDef};

fn main() {
    let repo_root = repo_root();
    let files = discover(&repo_root);

    // Pass 1: parse every file, build the backend module tree and per-module
    // use / re-export / definition tables, and collect raw references.
    let mut tree = Tree::default();
    let mut raw_refs: Vec<RawRef> = Vec::new();
    let mut macro_defs: Vec<MacroDef> = Vec::new();
    let mut parse_failures: Vec<PathBuf> = Vec::new();

    for f in &files {
        let Ok(src) = std::fs::read_to_string(&f.abs) else {
            parse_failures.push(f.abs.clone());
            continue;
        };
        let Ok(ast) = syn::parse_file(&src) else {
            parse_failures.push(f.abs.clone());
            continue;
        };
        let mut v = FileVisitor::new(f.base_mod.clone(), f.scope, f.source_crate.clone());
        v.visit_file(&ast);
        v.commit(&mut tree, &mut raw_refs, &mut macro_defs, &f.rel);
    }

    // Pass 2: resolve each reference to a definition crate and classify it.
    let mut records: Vec<RefRecord> = Vec::new();
    for r in &raw_refs {
        let resolution = match r.scope {
            ScopeKind::Backend | ScopeKind::BackendTest => {
                resolve_backend(&tree, &r.from_mod, &r.segs, 0)
            }
            ScopeKind::Floor | ScopeKind::FloorTest => {
                // Floor crates cannot name a target crate: crate::/super::/self::
                // stay inside the floor crate, and only an external moatless_*
                // head crosses a boundary.
                match r.segs.first().map(String::as_str) {
                    Some(h) if h != "crate" && h != "super" && h != "self" => {
                        external_crate(h).map_or(Resolution::Ignore, Resolution::ExternalCrate)
                    }
                    _ => Resolution::Ignore,
                }
            }
        };
        let target = match resolution {
            Resolution::Internal(c, _) => c,
            Resolution::ExternalCrate(c) => c,
            Resolution::Ignore | Resolution::Unresolved(_) => {
                if let Resolution::Unresolved(reason) = resolution {
                    records.push(RefRecord::unresolved(r, reason));
                }
                continue;
            }
        };
        if target == r.source_crate {
            continue; // same crate — not a cross-crate reference
        }
        let mut rec = RefRecord::edge(r, target);
        // A file-based `#[cfg(test)] mod ...;` (e.g. auth/test_support.rs,
        // adapters/registry/tests.rs) is parsed as its own production file, so its
        // test-only refs must be demoted here from the parent's cfg(test) gate.
        if tree.under_cfg_test(&r.from_mod) {
            rec.test_context = true;
        }
        records.push(rec);
    }

    let files_count = files.len();
    let membership = build_membership(&files);
    let report = build_report(&records, &macro_defs, files_count, &parse_failures, &membership);
    let baseline = build_baseline(&records);

    let out_dir = repo_root.join("scripts");
    let baseline_path = out_dir.join("crate-census-baseline.txt");
    let report_path = out_dir.join("crate-census-report.txt");

    // `--check` regenerates in memory and compares against the committed files
    // without writing them, so the CI ratchet needs no git repository (see
    // scripts/check-crate-census.sh). Default mode writes the files.
    if std::env::args().any(|a| a == "--check") {
        let mut ok = report_parse_failures(&parse_failures);
        ok &= check_current(&baseline_path, &baseline);
        ok &= check_current(&report_path, &report);
        if !ok {
            std::process::exit(1);
        }
        return;
    }

    write_file(&baseline_path, &baseline);
    write_file(&report_path, &report);

    // A parse failure means every reference in that file was silently dropped, so
    // the baseline it just wrote undercounts. Exit non-zero (the report names the
    // failed files) so the CI ratchet fails rather than passing on a partial scan.
    if !parse_failures.is_empty() {
        std::process::exit(1);
    }
}

/// Under `--check`: true when the committed file matches what we just generated;
/// otherwise names the stale file and returns false.
fn check_current(path: &Path, generated: &str) -> bool {
    if std::fs::read_to_string(path).unwrap_or_default() == generated {
        return true;
    }
    report_stale(&format!(
        "{} is stale — regenerate with `cargo run -p crate-census` and commit it.",
        path.display()
    ));
    false
}

/// Under `--check`: true when no file failed to parse; otherwise names the count
/// (the report lists the files) and returns false, so a partial scan cannot pass.
fn report_parse_failures(failures: &[PathBuf]) -> bool {
    if failures.is_empty() {
        return true;
    }
    report_stale(&format!(
        "{} file(s) failed to parse; the census is partial (see the report).",
        failures.len()
    ));
    false
}

#[expect(
    clippy::print_stderr,
    reason = "census --check: the stale artifact or parse failure must be named on stderr"
)]
fn report_stale(msg: &str) {
    eprintln!("crate-census: {msg}");
}

/// A reference collected in pass 1, before resolution.
pub struct RawRef {
    pub scope: ScopeKind,
    pub source_crate: String,
    pub from_mod: ModPath,
    pub segs: Vec<String>,
    pub class: RefClass,
    pub file: String,
    pub line: usize,
    pub in_test_cfg: bool,
}

struct DiscoveredFile {
    abs: PathBuf,
    rel: String,
    base_mod: ModPath,
    scope: ScopeKind,
    source_crate: String,
}

fn discover(root: &Path) -> Vec<DiscoveredFile> {
    let mut out = Vec::new();
    for (rel_root, scope) in CENSUS_ROOTS {
        let base = root.join(rel_root);
        if !base.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&base).into_iter().flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
            if rel.contains("/target/") {
                continue;
            }
            if let Some(df) = classify_file(root, p, &rel, *scope) {
                out.push(df);
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn classify_file(root: &Path, abs: &Path, rel: &str, scope: ScopeKind) -> Option<DiscoveredFile> {
    match scope {
        ScopeKind::Backend => {
            let inner = rel.strip_prefix("backend/src/")?;
            if inner == "main.rs" {
                return Some(DiscoveredFile {
                    abs: abs.to_path_buf(),
                    rel: rel.to_string(),
                    base_mod: vec!["\u{0}bin".to_string()],
                    scope,
                    source_crate: "server".to_string(),
                });
            }
            let base_mod = file_to_modpath(inner);
            let source_crate = crate_of_backend_mod(&base_mod).edge_crate().to_string();
            Some(DiscoveredFile {
                abs: abs.to_path_buf(),
                rel: rel.to_string(),
                base_mod,
                scope,
                source_crate,
            })
        }
        ScopeKind::BackendTest => Some(DiscoveredFile {
            abs: abs.to_path_buf(),
            rel: rel.to_string(),
            base_mod: Vec::new(),
            scope,
            source_crate: "test:backend".to_string(),
        }),
        ScopeKind::Floor | ScopeKind::FloorTest => {
            // crates/<name>/(src|tests)/...
            let inner = rel.strip_prefix("crates/")?;
            let mut parts = inner.split('/');
            let name = parts.next()?;
            // codex-sdk is excluded from the workspace (its own dep graph).
            if name == "codex-sdk" {
                return None;
            }
            let sub = parts.next()?;
            let scope = match sub {
                "src" => ScopeKind::Floor,
                "tests" => ScopeKind::FloorTest,
                _ => return None, // benches, examples, build.rs handled elsewhere
            };
            let source_crate = floor_crate_of_dir(name);
            let _ = root;
            Some(DiscoveredFile {
                abs: abs.to_path_buf(),
                rel: rel.to_string(),
                base_mod: Vec::new(),
                scope,
                source_crate,
            })
        }
    }
}

/// `task/reaper.rs` -> `["task","reaper"]`; `task/mod.rs` -> `["task"]`;
/// `lib.rs` -> `[]`.
fn file_to_modpath(inner: &str) -> ModPath {
    let stem = inner.strip_suffix(".rs").unwrap_or(inner);
    let stem = stem.strip_suffix("/mod").unwrap_or(stem);
    if stem == "lib" || stem == "mod" {
        return Vec::new();
    }
    stem.split('/').map(str::to_string).collect()
}

fn repo_root() -> PathBuf {
    // The tool lives at <root>/tools/crate-census; CARGO_MANIFEST_DIR points there.
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest)
        .ancestors()
        .nth(2)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

#[expect(clippy::expect_used, reason = "census tool: a failed output write must abort loudly")]
fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, contents).expect("write census output");
}

// -- report construction ---------------------------------------------------

fn build_baseline(records: &[RefRecord]) -> String {
    let fmt = |r: &RefRecord| {
        format!(
            "{:<22} -> {:<20} [{}] {}:{}  {}",
            r.source_crate, r.target_crate, r.class.tag(), r.file, r.line, r.path_text
        )
    };
    // The ratchet: sorted illegal production Cargo edges (CODE / ATTR / MACRO),
    // one per line. Doc paths are NOT here — they are never a Cargo edge, so an
    // illegal count that moved when a doc comment is edited would measure prose,
    // not architecture. They are listed in their own section below, out of the count.
    let mut illegal: BTreeSet<String> = BTreeSet::new();
    let mut doc_up: BTreeSet<String> = BTreeSet::new();
    for r in records {
        if !r.illegal || r.test_context {
            continue;
        }
        if r.class == RefClass::Doc {
            doc_up.insert(fmt(r));
        } else {
            illegal.insert(fmt(r));
        }
    }
    let mut s = String::new();
    s.push_str("# Illegal production Cargo edges: a CODE/ATTR/MACRO reference whose source band\n");
    s.push_str("# is <= its target band. This is the ratcheted count — it only shrinks. Doc paths\n");
    s.push_str("# are excluded (see the section below). Regenerate with `cargo run -p crate-census`.\n");
    let _ = writeln!(s, "# Illegal edge count: {}", illegal.len());
    for l in &illegal {
        s.push_str(l);
        s.push('\n');
    }
    // Intra-doc paths that point upward across the band order — real carve work
    // (an up-pointing `///` link stops resolving the moment the boundary exists,
    // as Step 8f showed), but not Cargo edges and never part of the count above.
    let _ = writeln!(s, "\n# --- intra-doc paths pointing up (NOT Cargo edges; carve work per Step 8f) ---");
    let _ = writeln!(s, "# Doc-path count (not in the illegal count above): {}", doc_up.len());
    for l in &doc_up {
        s.push_str(l);
        s.push('\n');
    }
    s
}

/// Row label for a top-level module that no arm of `crate_of_backend_mod` names.
const UNASSIGNED: &str = "UNASSIGNED";

/// Row label for a `#[cfg(test)]`-only backend-local module that ships in no target
/// crate (test scaffolding). Distinct from `UNASSIGNED`: it has no target crate
/// because it is test-only, which is a different fact from nobody having classified it.
const TEST_SCAFFOLD: &str = "TEST-SCAFFOLD (cfg(test), not carved)";

/// Per-crate membership of `backend/src`, generated from `bands.rs`: the top-level
/// modules each target crate owns and their combined line count. A module that
/// `crate_of_backend_mod` does not name lands in the `UNASSIGNED` row instead of
/// being folded into `server` — the reason it returns a distinct `Fallthrough`.
struct Membership {
    rows: Vec<MemberRow>,
}

struct MemberRow {
    crate_id: String,
    modules: BTreeSet<String>,
    lines: usize,
    files: usize,
}

fn build_membership(files: &[DiscoveredFile]) -> Membership {
    // crate id (or UNASSIGNED) -> (module display names, total lines, file count).
    let mut acc: BTreeMap<String, (BTreeSet<String>, usize, usize)> = BTreeMap::new();
    for f in files {
        if f.scope != ScopeKind::Backend {
            continue;
        }
        let crate_id = match crate_of_backend_mod(&f.base_mod) {
            CrateAssignment::Explicit(id) => id.to_string(),
            CrateAssignment::TestScaffold => TEST_SCAFFOLD.to_string(),
            CrateAssignment::Fallthrough => UNASSIGNED.to_string(),
        };
        let lines = std::fs::read_to_string(&f.abs)
            .map_or(0, |s| s.bytes().filter(|&b| b == b'\n').count());
        let e = acc.entry(crate_id).or_default();
        e.0.insert(membership_module_name(&f.rel));
        e.1 += lines;
        e.2 += 1;
    }

    // Emit in band-carve order, then UNASSIGNED, then any leftover (unexpected).
    let mut rows: Vec<MemberRow> = Vec::new();
    let mut take = |acc: &mut BTreeMap<String, (BTreeSet<String>, usize, usize)>, id: &str| {
        let (modules, lines, files) = acc.remove(id).unwrap_or_default();
        rows.push(MemberRow { crate_id: id.to_string(), modules, lines, files });
    };
    for (_step, crate_id) in CARVE_ORDER {
        take(&mut acc, crate_id);
    }
    take(&mut acc, TEST_SCAFFOLD);
    take(&mut acc, UNASSIGNED);
    for (crate_id, (modules, lines, files)) in acc {
        rows.push(MemberRow { crate_id, modules, lines, files });
    }
    Membership { rows }
}

/// The display name for a file's owning module, shown at the depth that decides its
/// crate: one path segment normally, but `sandbox/*` and non-provider `adapters/*`
/// split on the second and `adapters/providers/*` on the third, so they are shown at
/// that depth (else three sandbox crates all read `sandbox/`). A prefix with files
/// beneath it renders as `a/b/`; a leaf file keeps its name (`config.rs`).
fn membership_module_name(rel: &str) -> String {
    let Some(rest) = rel.strip_prefix("backend/src/") else {
        return rel.to_string();
    };
    let comps: Vec<&str> = rest.split('/').collect();
    let depth = match (comps.first().copied(), comps.get(1).copied()) {
        (Some("adapters"), Some("providers")) => 3,
        (Some("adapters" | "sandbox"), _) => 2,
        _ => 1,
    }
    .min(comps.len());
    if comps.len() > depth {
        format!("{}/", comps[..depth].join("/")) // a directory prefix
    } else {
        comps[..depth].join("/") // the leaf file itself
    }
}

fn write_membership(s: &mut String, m: &Membership) {
    let _ = writeln!(s, "== CRATE MEMBERSHIP (backend/src -> target crate; generated from bands.rs) ==");
    let _ = writeln!(s, "  Line counts are newline counts (wc -l). This table and its TOTAL count what has");
    let _ = writeln!(s, "  NOT yet been carved: a crate already extracted (core, adapter-model) has left");
    let _ = writeln!(s, "  backend/src and has no row here, so a shrinking TOTAL is carve progress, not loss.");
    let _ = writeln!(s, "  TEST-SCAFFOLD collects cfg(test)-only backend-local modules that ship in no crate");
    let _ = writeln!(s, "  and are never carved. UNASSIGNED collects any top-level module crate_of_backend_mod");
    let _ = writeln!(s, "  does not name — a finding to resolve in bands.rs, not a --check failure; it is empty");
    let _ = writeln!(s, "  when every module is placed.");
    let mut total = 0usize;
    for row in &m.rows {
        total += row.lines;
        let mods = if row.modules.is_empty() {
            "(none)".to_string()
        } else {
            row.modules.iter().cloned().collect::<Vec<_>>().join(" ")
        };
        let _ = writeln!(s, "\n  {}  ({} files, {} lines)", row.crate_id, row.files, row.lines);
        let _ = writeln!(s, "    {mods}");
    }
    let _ = writeln!(s, "\n  TOTAL backend/src: {total} lines");
    let _ = writeln!(s);
}

#[allow(clippy::too_many_lines)]
fn build_report(
    records: &[RefRecord],
    macro_defs: &[MacroDef],
    files_count: usize,
    parse_failures: &[PathBuf],
    membership: &Membership,
) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "CRATE-CENSUS REPORT");
    let _ = writeln!(s, "files parsed: {files_count}");
    let _ = writeln!(s, "parse failures: {}", parse_failures.len());
    for p in parse_failures {
        let _ = writeln!(s, "  FAILED {}", p.display());
    }
    let _ = writeln!(s);

    // Totals by class and legality, production only unless noted.
    let mut prod_code_legal = 0usize;
    let mut prod_code_illegal = 0usize;
    let mut attr_legal = 0usize;
    let mut attr_illegal = 0usize;
    let mut doc_total = 0usize;
    let mut macro_legal = 0usize;
    let mut macro_illegal = 0usize;
    let mut test_ctx = 0usize;
    let mut to_floor = 0usize;
    let mut unresolved = 0usize;

    for r in records {
        if r.unresolved {
            unresolved += 1;
            continue;
        }
        if r.class == RefClass::Doc {
            doc_total += 1;
            continue;
        }
        if r.test_context {
            test_ctx += 1;
            continue;
        }
        if r.to_floor {
            to_floor += 1;
        }
        match r.class {
            RefClass::Code => {
                if r.illegal {
                    prod_code_illegal += 1;
                } else {
                    prod_code_legal += 1;
                }
            }
            RefClass::Attribute => {
                if r.illegal {
                    attr_illegal += 1;
                } else {
                    attr_legal += 1;
                }
            }
            RefClass::Macro => {
                if r.illegal {
                    macro_illegal += 1;
                } else {
                    macro_legal += 1;
                }
            }
            RefClass::Doc => {}
        }
    }

    let _ = writeln!(s, "== TOTALS (production, cross-crate) ==");
    let _ = writeln!(s, "CODE       legal {prod_code_legal:>5}  illegal {prod_code_illegal:>4}");
    let _ = writeln!(
        s,
        "ATTRIBUTE  legal {attr_legal:>5}  illegal {attr_illegal:>4}   (utoipa/derive/etc. token streams)"
    );
    let _ = writeln!(
        s,
        "MACRO      legal {macro_legal:>5}  illegal {macro_illegal:>4}   (function-like macro-invocation token streams)"
    );
    let _ = writeln!(s, "DOC        {doc_total:>5}  (intra-doc paths — never a Cargo edge, never illegal)");
    let _ = writeln!(s, "  of the legal CODE edges, {to_floor} point down into the existing crates/* floor");
    let _ = writeln!(s, "TEST-CONTEXT {test_ctx:>4}  (backend/tests, crates/*/tests, #[cfg(test)] mods — dev-dep, not band-bound)");
    let _ = writeln!(s, "UNRESOLVED  {unresolved:>5}  (glob-through-facade / prelude / macro-var — audited below)");
    let _ = writeln!(s);

    write_membership(&mut s, membership);

    // Illegal edge classes (production), grouped by (source -> target), with the
    // step each is pinned to (filled from the pin table).
    let mut classes: BTreeMap<(String, String), Vec<&RefRecord>> = BTreeMap::new();
    for r in records {
        if r.illegal && !r.test_context && r.class != RefClass::Doc {
            classes.entry((r.source_crate.clone(), r.target_crate.clone())).or_default().push(r);
        }
    }
    let _ = writeln!(s, "== ILLEGAL PRODUCTION EDGE CLASSES ({} classes) ==", classes.len());
    let mut unpinned_sites: BTreeSet<String> = BTreeSet::new();
    for ((src, dst), refs) in &classes {
        let _ = writeln!(s, "\n{src} -> {dst}   x{}", refs.len());
        let mut by_site: BTreeSet<String> = BTreeSet::new();
        for r in refs {
            let pin = pin_for(src, dst, &r.path_text);
            by_site.insert(format!(
                "    {}:{}  [{}] {}   -> [{}]",
                r.file, r.line, r.class.tag(), r.path_text, pin
            ));
            if pin == "UNPINNED" {
                unpinned_sites.insert(format!("  {src} -> {dst}   {}   ({}:{})", r.path_text, r.file, r.line));
            }
        }
        for site in by_site {
            let _ = writeln!(s, "{site}");
        }
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "== UNPINNED ILLEGAL REFERENCES (headline) — {} ==", unpinned_sites.len());
    if unpinned_sites.is_empty() {
        let _ = writeln!(s, "  none");
    } else {
        for site in &unpinned_sites {
            let _ = writeln!(s, "{site}");
        }
    }
    let _ = writeln!(s);

    // Unresolved references, grouped by reason — the audit the TOTALS promises.
    // These resolve to no definite crate (internal-glob ambiguity, a path above
    // the crate root, or the depth guard). None is scored illegal; listed so a
    // reader can confirm none hides a higher-band edge.
    let mut unres: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in records {
        if r.unresolved {
            let reason = r.target_crate.trim_start_matches('?').to_string();
            unres.entry(reason).or_default().insert(format!(
                "    {} :: {}   ({}:{})",
                r.source_crate, r.path_text, r.file, r.line
            ));
        }
    }
    let _ = writeln!(s, "== UNRESOLVED REFERENCES (audit) ==");
    for (reason, sites) in &unres {
        let _ = writeln!(s, "  [{reason}] x{}", sites.len());
        for site in sites {
            let _ = writeln!(s, "{site}");
        }
    }
    let _ = writeln!(s);

    // Carve readiness + Tarjan on the target-crate graph.
    write_carve_analysis(&mut s, records);

    // Macro inventory over census scope.
    let _ = writeln!(s, "== macro_rules! INVENTORY (census scope) ==");
    for m in macro_defs {
        let _ = writeln!(
            s,
            "  {}  {}  emits-cross-module-path: {}",
            m.file,
            m.name,
            if m.emits_path { "YES" } else { "no" }
        );
    }
    s
}

/// The band order carve sequence (10a-10i) and the crate each step extracts.
const CARVE_ORDER: &[(&str, &str)] = &[
    // 10a/core is carved (crates/core); it is scored via external_crate now, not
    // carried here, so the frontier advances to identity.
    ("10b", "identity"),
    ("10c", "workspaces"),
    ("10d", "tasks"),
    ("10e", "sandbox-control"),
    ("10f", "sandbox-k8s"),
    ("10f", "sandbox-docker"),
    ("10g", "loops"),
    ("10h", "adapter-github-pr"),
    ("10h", "adapter-linear"),
    ("10h", "adapter-slack"),
    ("10h", "adapter-telegram"),
    ("10h", "adapter-webhook"),
    ("10i", "server"),
];

/// Per-crate carve readiness (illegal outgoing edges block a carve) and a Tarjan
/// SCC pass on the production target-crate graph.
fn write_carve_analysis(s: &mut String, records: &[RefRecord]) {
    // Illegal outgoing edges per source crate (these block its carve).
    let mut blockers: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    // Production target-crate edge sets for Tarjan: the full graph, and the
    // legal-only graph (illegal edges removed).
    let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut adj_legal: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in records {
        if r.unresolved || r.test_context || r.class == RefClass::Doc {
            continue;
        }
        if r.source_crate.starts_with("floor:") || r.target_crate.starts_with("floor:") {
            continue;
        }
        adj.entry(r.source_crate.clone()).or_default().insert(r.target_crate.clone());
        if r.illegal {
            *blockers.entry(&r.source_crate).or_default().entry(&r.target_crate).or_default() += 1;
        } else {
            adj_legal.entry(r.source_crate.clone()).or_default().insert(r.target_crate.clone());
        }
    }

    // Legal-dependency ordering: a crate is carvable only when it has no illegal
    // outgoing edge AND every crate it depends on is already extracted. Carves go
    // lowest band first, so the readiness of every crate but the base case is a
    // frontier to re-run after each carve, not a standalone verdict. `no illegal
    // outgoing` alone is necessary, not sufficient — sandbox-k8s and server have
    // none yet still depend on crates that are not carved, so they read WAITING.
    let legal_deps = |c: &str| -> Vec<String> {
        adj_legal.get(c).map(|d| d.iter().cloned().collect()).unwrap_or_default()
    };
    // Only the 15 crates in CARVE_ORDER are still inside the backend. A legal dep
    // on anything else (the `crates/*` floor, `adapter-model`) is on a crate that
    // already exists, so it is satisfied from the start — a dep only blocks a carve
    // while it is itself an un-carved carve target.
    let carve_set: BTreeSet<&str> = CARVE_ORDER.iter().map(|(_, c)| *c).collect();
    let dep_pending = |c: &str, carved: &BTreeSet<String>| -> Vec<String> {
        legal_deps(c)
            .into_iter()
            .filter(|d| carve_set.contains(d.as_str()) && !carved.contains(d))
            .collect()
    };
    // Topological order over the legal DAG (lowest band first): the sequence
    // carves must follow. Nothing is carved yet, so this is the frontier unrolled.
    let mut carved: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();
    loop {
        let mut progressed = false;
        for (_step, crate_id) in CARVE_ORDER {
            let id = (*crate_id).to_string();
            if carved.contains(&id) {
                continue;
            }
            if dep_pending(crate_id, &carved).is_empty() {
                carved.insert(id.clone());
                order.push(id);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }

    let _ = writeln!(s, "== CARVE READINESS (frontier — legal-dependency order, lowest band first) ==");
    let _ = writeln!(s, "  READY = no illegal outgoing edge AND every crate it depends on is already");
    let _ = writeln!(s, "  extracted. Nothing is extracted yet, so a crate whose legal dependencies are");
    let _ = writeln!(s, "  all still in the backend reads WAITING (necessary but not sufficient), not");
    let _ = writeln!(s, "  READY. Re-run after each carve; the frontier is the first crate below that is");
    let _ = writeln!(s, "  READY. The base case is the lowest-band crate, which depends only on the floor.");
    let frontier = order.first().cloned().unwrap_or_default();
    let frontier_illegal: usize = blockers.get(frontier.as_str()).map_or(0, |e| e.values().sum());
    if frontier_illegal == 0 {
        let _ = writeln!(s, "  FRONTIER -> {frontier}: READY now (no illegal outgoing, floor-only deps).");
    } else {
        let _ = writeln!(s, "  FRONTIER -> {frontier}: the base case, BLOCKED on {frontier_illegal} illegal outgoing. No crate is carvable until it clears.");
    }
    let _ = writeln!(s);
    let empty: BTreeSet<String> = BTreeSet::new();
    for crate_id in &order {
        // Deps still inside the backend at instant zero (nothing carved yet).
        let pending = dep_pending(crate_id, &empty);
        let dep_str = if pending.is_empty() {
            "floor / adapter-model only".to_string()
        } else {
            pending.join(", ")
        };
        let illegal = blockers.get(crate_id.as_str());
        let illegal_total: usize = illegal.map_or(0, |e| e.values().sum());
        let state = if illegal_total > 0 {
            let targets: Vec<String> =
                illegal.map(|e| e.iter().map(|(t, n)| format!("{t}(x{n})")).collect()).unwrap_or_default();
            format!("BLOCKED — {illegal_total} illegal outgoing to {}", targets.join(", "))
        } else if pending.is_empty() {
            "READY".to_string()
        } else {
            "WAITING — no illegal outgoing, but its deps are not extracted yet".to_string()
        };
        let _ = writeln!(s, "  {crate_id:<18} deps: {dep_str:<44} {state}");
    }
    let _ = writeln!(s);

    // Tarjan SCC over the directed target-crate graph, full then legal-only.
    let _ = writeln!(s, "== TARJAN SCCs (production target-crate graph) ==");
    write_sccs(s, "full graph (legal + illegal edges)", &tarjan(&adj));
    write_sccs(s, "legal edges only (illegal removed)", &tarjan(&adj_legal));
    let _ = writeln!(s);
}

fn write_sccs(s: &mut String, label: &str, sccs: &[Vec<String>]) {
    let cycles: Vec<&Vec<String>> = sccs.iter().filter(|c| c.len() > 1).collect();
    if cycles.is_empty() {
        let _ = writeln!(s, "  {label}: DAG — no multi-crate cycle");
    } else {
        for c in &cycles {
            let _ = writeln!(s, "  {label}: CYCLE ({} crates) {}", c.len(), c.join(" <-> "));
        }
    }
}

/// Tarjan strongly-connected components.
fn tarjan(adj: &std::collections::BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    struct State<'a> {
        adj: &'a std::collections::BTreeMap<String, BTreeSet<String>>,
        index: std::collections::HashMap<String, u32>,
        low: std::collections::HashMap<String, u32>,
        on_stack: std::collections::HashSet<String>,
        stack: Vec<String>,
        counter: u32,
        out: Vec<Vec<String>>,
    }
    fn strong(st: &mut State, v: &str) {
        st.index.insert(v.to_string(), st.counter);
        st.low.insert(v.to_string(), st.counter);
        st.counter += 1;
        st.stack.push(v.to_string());
        st.on_stack.insert(v.to_string());
        if let Some(succ) = st.adj.get(v) {
            for w in succ {
                if !st.index.contains_key(w) {
                    strong(st, w);
                    let lw = st.low[w];
                    let e = st.low.get_mut(v).unwrap_or_else(|| unreachable!());
                    *e = (*e).min(lw);
                } else if st.on_stack.contains(w) {
                    let iw = st.index[w];
                    let e = st.low.get_mut(v).unwrap_or_else(|| unreachable!());
                    *e = (*e).min(iw);
                }
            }
        }
        if st.low[v] == st.index[v] {
            let mut comp = Vec::new();
            while let Some(w) = st.stack.pop() {
                st.on_stack.remove(&w);
                let done = w == v;
                comp.push(w);
                if done {
                    break;
                }
            }
            st.out.push(comp);
        }
    }
    let mut nodes: BTreeSet<String> = adj.keys().cloned().collect();
    for succ in adj.values() {
        for w in succ {
            nodes.insert(w.clone());
        }
    }
    let mut st = State {
        adj,
        index: std::collections::HashMap::new(),
        low: std::collections::HashMap::new(),
        on_stack: std::collections::HashSet::new(),
        stack: Vec::new(),
        counter: 0,
        out: Vec::new(),
    };
    for n in &nodes {
        if !st.index.contains_key(n) {
            strong(&mut st, n);
        }
    }
    st.out
}

/// Pins an illegal reference to the step that removes it, per
/// `.plans/backend-ports-and-crates.md`. Pins are per-reference, not per-class:
/// a class such as `identity -> server` mixes an AppState edge (Step 5) with a
/// setup edge (8h). Anything this returns as UNPINNED is the headline — an edge
/// no step in the plan removes.
fn pin_for(src: &str, dst: &str, path: &str) -> &'static str {
    // Step 5 — the AppState decomposition this task is named for. Every
    // `crate::state::AppState` reference from a non-server band is a Step-5 port.
    if path.contains("state::AppState") {
        return "Step 5 (AppState -> ports)";
    }
    // Ruling A — the LeaderGuard port already exists (leader.rs:1-4). The codex
    // refresher bypasses it (workspaces -> sandbox-k8s x3, a Step-3 residue); the
    // schedule_runner uses it correctly but leader.rs is a root file the table maps
    // to server, so it reads loops -> server. Move the trait to core; four edges die.
    if path.contains("leader::LeaderGuard") {
        return "Ruling A — LeaderGuard trait -> core (leader.rs band error)";
    }
    if src == "workspaces" && dst == "sandbox-k8s" {
        return "Ruling A — codex refresher bypasses LeaderGuard port (Step-3 residue)";
    }
    // Ruling B — the eleven sandbox-control -> provider refs are the 10f defect
    // repriced: non-Kubernetes helpers (workspace-config parsing, failure-reason
    // strings, StorageBackend) that live under k8s/ but belong in sandbox-control.
    if src == "sandbox-control"
        && (path.contains("auto_provision::")
            || path.contains("pod_status::")
            || path.contains("storage::StorageBackend"))
    {
        return "10f (repriced) — non-k8s helpers under k8s/, move to sandbox-control";
    }
    // E-(b) — auth/routes.rs's session-gate TaskDao holding: verify_handler's
    // preview-URL access check holds `Arc<dyn TaskDao>` to gate a proxy request.
    // Ruling C did not touch it (its TaskDao moves were intra-tasks); identity-only,
    // since no core -> tasks TaskDao remains. Priced two ways, awaiting ruling.
    if src == "identity" && dst == "tasks" && path.contains("task::dao::TaskDao") {
        return "E-(b) — auth/routes session-gate TaskDao holding";
    }
    // Ruling D — 9e is reopened: the shell-reconcile hook is a consumer-declared
    // port (moatless-tasks declares, adapter-github-pr registers at the root), the
    // AgentErrorSink shape, not the Adapter-trait method that shell_reconcile.rs
    // refuses. Still open, still blocking 10d.
    if src == "tasks" && dst == "adapter-github-pr" {
        return "9e (reopened) — consumer-declared hook; AgentErrorSink precedent";
    }
    // 8b scope enums + their remainder + the error/credential cluster.
    if src == "identity" && dst == "workspaces" {
        return "8b + error/credential cluster";
    }
    if path.contains("ProviderSettingsError") {
        return "8d error half / error cluster";
    }
    // 8h — the setup split, deferred until SetupRequirementCheck earns a 2nd impl.
    if path.contains("setup::") {
        return "8h (deferred)";
    }
    // registry <-> providers: 7d field-widening + 10h provider carve.
    if src == "loops" && dst.starts_with("adapter-") {
        return "7d + 10h — registry field-widening + provider carve";
    }
    // 10f — the only real sibling edge, the misfiled sandbox helpers.
    if src == "sandbox-docker" && dst == "sandbox-k8s" {
        return "10f — misfiled sandbox helpers";
    }
    // 8c's real content — the tasks/message -> loops consumer-owned port.
    if src == "tasks" && dst == "loops" {
        return "8c-port — tasks/message -> loops";
    }
    // 10d — invert the reaper's (and its siblings') task -> sandbox concretes.
    // The `sandbox::secrets` constants are a placement move to core, like 8e.
    if src == "tasks" && (dst == "sandbox-control" || dst == "sandbox-k8s") {
        if path.contains("sandbox::secrets") {
            return "sandbox::secrets placement -> core (like 8e)";
        }
        return "10d — invert task -> sandbox concretes to ports";
    }
    // 9d / 10g — the repository-sync server composition.
    if src == "workspaces" && dst == "loops" {
        return "9d / 10g — server-band sync composition";
    }
    // 9c / 10c — the credential broadcast fan-out refactor.
    if src == "workspaces" && dst == "tasks" {
        return "9c / 10c — broadcast fan-out refactor";
    }
    // 9b remainder — the GITHUB_TOKEN secret name, a placement move to core.
    if path.contains("sandbox::secrets::GITHUB_TOKEN") {
        return "9b remainder — GITHUB_TOKEN placement -> core";
    }
    // The composition-root DI container named by a lower band — same family as
    // AppState (Step 5): the composition root reaching down.
    if path.contains("compose::Ports") {
        return "Step 5 family — composition root (Ports)";
    }
    "UNPINNED"
}

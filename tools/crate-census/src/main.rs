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

use bands::{band_of, crate_of_backend_mod, external_crate, floor_crate_of_dir, CrateAssignment, ScopeKind, CENSUS_ROOTS};
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
    let extracted = extracted_crates(&files);
    let unbanded = carved_but_unbanded(&files);
    let report = build_report(
        &records,
        &macro_defs,
        files_count,
        &parse_failures,
        &membership,
        &extracted,
        &unbanded,
    );

    let out_dir = repo_root.join("scripts");
    let baseline_path = out_dir.join("crate-census-baseline.txt");
    let report_path = out_dir.join("crate-census-report.txt");

    // The illegal edge set and its current size. The count is bounded by the
    // ratchet CEILING on the committed baseline's `# Illegal edge count:` line:
    // the baseline is regenerated with `min(committed_ceiling, current_count)`,
    // so regeneration lowers the ceiling to the current count but NEVER raises
    // it. A branch that adds an illegal edge therefore cannot make the gate green
    // by regenerating — the ceiling stays put and `check_ceiling` rejects the
    // excess. Absent a committed baseline (first run), bootstrap to the current
    // count: there is nothing to ratchet against yet.
    let (illegal, doc_up) = partition_edges(&records);
    let current_count = illegal.len();
    let committed_ceiling = read_committed_ceiling(&baseline_path).unwrap_or(current_count);
    let effective_ceiling = committed_ceiling.min(current_count);
    let baseline = build_baseline(&illegal, &doc_up, effective_ceiling);

    // `--check` regenerates in memory and compares against the files ON DISK
    // (the working tree) without writing them and without invoking git, so the
    // CI ratchet needs no git repository (see scripts/check-crate-census.sh).
    // Because the comparison is against the working tree, a regenerated-but-
    // uncommitted baseline or report makes `--check` pass locally while CI,
    // which runs against the freshly checked-out committed tree, fails — so
    // regenerate and commit BOTH files and run the check against a clean tree.
    // Default mode writes the files.
    if std::env::args().any(|a| a == "--check") {
        let mut ok = report_parse_failures(&parse_failures);
        ok &= check_current(&baseline_path, &baseline);
        ok &= check_current(&report_path, &report);
        // The monotonic ratchet, independent of the byte-compare above: the
        // current count must not exceed the committed ceiling. This is the gate
        // the byte-compare cannot be — regeneration keeps the committed file
        // self-consistent at any count, so only an assertion against the
        // ceiling catches a carve regression.
        ok &= check_ceiling(current_count, committed_ceiling, &illegal, &baseline_path);
        if !ok {
            std::process::exit(1);
        }
        return;
    }

    // Say out loud what the ceiling did, so a moved ratchet is observed rather
    // than inferred from a silent diff. A carve lowers it; a regression that
    // raised the count is pinned at the old ceiling and warned about here (the
    // subsequent `--check` is what fails).
    if effective_ceiling < committed_ceiling {
        announce(&format!(
            "illegal-edge ceiling {committed_ceiling} -> {effective_ceiling} (lowered by this regeneration)"
        ));
    } else if current_count > committed_ceiling {
        announce(&format!(
            "illegal-edge count {current_count} exceeds the committed ceiling {committed_ceiling}; \
             the baseline pins the ceiling at {committed_ceiling} (regeneration never raises it), so \
             `--check` will fail until the edge is removed — see its output before raising the ceiling."
        ));
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

/// An informational note about the ratchet (a lowered or over-ceiling count),
/// on stderr so it stays out of any capture of the tool's file output.
#[expect(
    clippy::print_stderr,
    reason = "census: a moved ratchet ceiling must be observable, not silent"
)]
fn announce(msg: &str) {
    eprintln!("crate-census: {msg}");
}

/// Parse the committed ceiling from `crate-census-baseline.txt` — the integer on
/// its `# Illegal edge count: N` line. `None` when the file or that line is
/// absent, in which case the caller bootstraps to the current count (there is
/// nothing to ratchet against yet). Only the count line is read, so it survives
/// changes to the surrounding header prose.
fn read_committed_ceiling(path: &Path) -> Option<usize> {
    parse_ceiling(&std::fs::read_to_string(path).ok()?)
}

/// The pure parse behind [`read_committed_ceiling`] — the integer on the
/// `# Illegal edge count: N` line, ignoring the surrounding prose.
fn parse_ceiling(content: &str) -> Option<usize> {
    content
        .lines()
        .find_map(|l| l.strip_prefix("# Illegal edge count:"))
        .and_then(|n| n.trim().parse().ok())
}

/// The illegal edges listed in the committed baseline: every non-comment,
/// non-blank line above the intra-doc section. Used to name exactly the edges a
/// branch added when its count exceeds the ceiling. The line format matches
/// [`partition_edges`], so the two sets diff cleanly.
fn read_committed_illegal_edges(path: &Path) -> BTreeSet<String> {
    parse_illegal_edges(&std::fs::read_to_string(path).unwrap_or_default())
}

/// The pure parse behind [`read_committed_illegal_edges`] — every non-comment,
/// non-blank line above the intra-doc section, in the [`partition_edges`] format
/// so the two sets diff cleanly.
fn parse_illegal_edges(content: &str) -> BTreeSet<String> {
    content
        .split("# --- intra-doc paths")
        .next()
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// The monotonic ratchet: the current illegal count must not exceed the committed
/// ceiling. Regeneration cannot raise the ceiling, so a failure here means the
/// branch added an edge (or reverted a carve). Names the new edges and — per the
/// gate's whole purpose — pre-empts the wrong fix: regenerating will not clear
/// it, and raising the ceiling is a deliberate architecture decision, not a
/// routine regenerate. Modelled on the repo's best fail message (the helm
/// `apiServerCidrs` render failure): what broke, the fix, and the wrong fix named.
fn check_ceiling(current: usize, ceiling: usize, illegal: &BTreeSet<String>, path: &Path) -> bool {
    let committed = read_committed_illegal_edges(path);
    match ceiling_violation(current, ceiling, illegal, &committed) {
        Some(msg) => {
            report_stale(&msg);
            false
        }
        None => true,
    }
}

/// The pure ratchet decision behind [`check_ceiling`]: `None` when the current
/// count is within the ceiling, else the failure message. `committed` is the
/// baseline's on-disk edge list, used to name exactly the edges a branch added.
/// Kept free of IO and printing so the message can be asserted in tests.
fn ceiling_violation(
    current: usize,
    ceiling: usize,
    illegal: &BTreeSet<String>,
    committed: &BTreeSet<String>,
) -> Option<String> {
    if current <= ceiling {
        return None;
    }
    let new_edges: Vec<&String> = illegal.iter().filter(|e| !committed.contains(*e)).collect();
    let mut msg = format!(
        "illegal cross-band edge count rose to {current}, above the ratcheted ceiling of {ceiling}. \
         The census ratchet only shrinks — every carve lowers it — so an increase is a carve regression.\n"
    );
    // Name the added edges when the committed baseline still holds the old list
    // (the author added an edge but did not regenerate). Once regenerated, the
    // committed list already contains them, so — with no git in this tool — the
    // excess cannot be named from files alone; point at the diff a human can run.
    if new_edges.is_empty() {
        let _ = writeln!(
            msg,
            "  The committed baseline already lists all {current} edges (it was regenerated), so the \
             {} over the ceiling cannot be named from files alone. Run \
             `git diff origin/main -- scripts/crate-census-baseline.txt` to see the added edge(s).",
            current - ceiling
        );
    } else {
        let _ = writeln!(msg, "  {} new illegal edge(s) not in the committed baseline:", new_edges.len());
        for e in &new_edges {
            let _ = writeln!(msg, "    {e}");
        }
    }
    msg.push_str(
        "  Regenerating will NOT fix this: `cargo run -p crate-census` pins the ceiling at the \
         committed value and never raises it. Remove the edge (route through a port, or move the \
         item down a band) so the count returns to the ceiling. If the increase is genuinely \
         intended, raising the `# Illegal edge count:` line in scripts/crate-census-baseline.txt is \
         a deliberate architecture decision — put the reasoning in the commit message and expect a \
         reviewer to weigh it.",
    );
    Some(msg)
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

/// Partition the resolved records into the sorted, formatted illegal production
/// Cargo edges and the up-pointing intra-doc paths. `illegal` is the ratcheted
/// set — its length is the current illegal count the baseline's ceiling bounds.
/// Doc paths are kept out of it: they are never a Cargo edge, so an illegal
/// count that moved when a doc comment is edited would measure prose, not
/// architecture. They are listed in their own baseline section, out of the count.
fn partition_edges(records: &[RefRecord]) -> (BTreeSet<String>, BTreeSet<String>) {
    let fmt = |r: &RefRecord| {
        format!(
            "{:<22} -> {:<20} [{}] {}:{}  {}",
            r.source_crate, r.target_crate, r.class.tag(), r.file, r.line, r.path_text
        )
    };
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
    (illegal, doc_up)
}

/// Render `crate-census-baseline.txt`. The header count is the ratchet CEILING,
/// not `illegal.len()`: the caller passes `min(committed_ceiling, current_count)`
/// so a regeneration can only ever lower it. When a branch adds an illegal edge
/// the ceiling stays put and the header sits below the edge-list length — a
/// state `--check`'s [`check_ceiling`] rejects until the edge goes or the ceiling
/// is raised by hand.
fn build_baseline(illegal: &BTreeSet<String>, doc_up: &BTreeSet<String>, ceiling: usize) -> String {
    let mut s = String::new();
    s.push_str("# Illegal production Cargo edges: a CODE/ATTR/MACRO reference whose source band\n");
    s.push_str("# is <= its target band. The count below is a ratchet CEILING enforced by `--check`:\n");
    s.push_str("# regeneration lowers it to the current count but never raises it, so a branch that\n");
    s.push_str("# adds an edge fails the check until the edge is removed — or the ceiling is raised by\n");
    s.push_str("# hand as a deliberate, reviewed decision. Doc paths are excluded (see the section\n");
    s.push_str("# below). Regenerate with `cargo run -p crate-census`.\n");
    let _ = writeln!(s, "# Illegal edge count: {ceiling}");
    for l in illegal {
        s.push_str(l);
        s.push('\n');
    }
    // Intra-doc paths that point upward across the band order — real carve work
    // (an up-pointing `///` link stops resolving the moment the boundary exists,
    // as Step 8f showed), but not Cargo edges and never part of the count above.
    let _ = writeln!(s, "\n# --- intra-doc paths pointing up (NOT Cargo edges; carve work per Step 8f) ---");
    let _ = writeln!(s, "# Doc-path count (not in the illegal count above): {}", doc_up.len());
    for l in doc_up {
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

/// The banded crates already carved into `crates/*`, derived from the scan rather
/// than hand-listed. A `crates/<dir>` resolves through `floor_crate_of_dir` to its
/// real band id only once it is a modelled crate (the carve adds the arm, as #385
/// did for `identity`) and to `floor:<dir>` otherwise — so a Floor-scope file whose
/// source crate is not a `floor:` id is a crate that both exists on disk (it was
/// scanned) and is banded (it is measured). That pair IS extraction: the readiness
/// table keys on it so a carved crate reads EXTRACTED, not READY, and its dependents
/// flip to READY. A hardcoded "identity is done" list is exactly what this avoids.
fn extracted_crates(files: &[DiscoveredFile]) -> BTreeSet<String> {
    files
        .iter()
        .filter(|f| matches!(f.scope, ScopeKind::Floor | ScopeKind::FloorTest))
        .map(|f| f.source_crate.clone())
        .filter(|c| !c.starts_with("floor:"))
        .collect()
}

/// Crate dirs physically extracted under `crates/<dir>` whose `<dir>` names a real
/// carve-target band (`band_of` in `1..=10`) but which still resolved to
/// `floor:<dir>` — the `bands.rs` `floor_crate_of_dir` arm was never added (the
/// #385-style precursor step was skipped). Such a crate scores as the floor:
/// excluded from every band comparison, so its 0-in/0-out property stops being
/// measured and a future illegal edge into or out of it is never caught — a whole
/// carved crate silently absent from the ratchet. The extraction signal reads its
/// files as floor (they never appear in `extracted_crates`), so nothing else
/// surfaces the omission; this row does.
///
/// `band_of` returns 0 for any `floor:`-prefixed id (its floor guard fires before
/// the match), so the prefix is stripped BEFORE the lookup — otherwise every floor
/// crate would read band 0 and this guard could never fire. The real floor crates
/// (`floor:events`, `floor:api-schemas`, …) name nothing in `1..=10`, so they do
/// not trip it: zero false positives until a target-band crate is carved without
/// its arm. The names it returns are the stripped dir names, ready to print.
fn carved_but_unbanded(files: &[DiscoveredFile]) -> BTreeSet<String> {
    files
        .iter()
        .filter(|f| matches!(f.scope, ScopeKind::Floor | ScopeKind::FloorTest))
        .filter_map(|f| f.source_crate.strip_prefix("floor:"))
        .filter(|name| (1..=10).contains(&band_of(name)))
        .map(str::to_string)
        .collect()
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

/// The membership table's header. Derived from the live `extracted` set and the row
/// crate ids rather than hardcoded, so the prose cannot drift out of step with the
/// table the way a hand-written "(core, adapter-model) has no row" line did once
/// identity and workspaces began leaving permanent facade stubs in `backend/src`: an
/// extracted crate is enumerated as keeping a facade row or as having left no stub
/// according to whether `build_membership` actually produced a row for it, and the
/// two states print on separate lines so a test can assert each crate lands on the
/// right one. The invariant
/// `membership_header_lists_each_extracted_crate_under_its_facade_state` pins: an
/// extracted crate with a row appears on the keeps-a-facade-row line and not the
/// no-stub line, and vice versa.
fn membership_header(m: &Membership, extracted: &BTreeSet<String>) -> String {
    // `files > 0` is the facade-stub test, not mere row presence: build_membership
    // emits a zero-file placeholder row for every CARVE_ORDER crate (take() does
    // acc.remove(id).unwrap_or_default()), so a carved target that left no facade
    // files still has a row. Counting that as "keeps a facade row" would recreate
    // the exact stale-prose bug this header derivation exists to prevent.
    let row_ids: BTreeSet<&str> =
        m.rows.iter().filter(|r| r.files > 0).map(|r| r.crate_id.as_str()).collect();
    let with_row: Vec<&str> = extracted.iter().map(String::as_str).filter(|c| row_ids.contains(c)).collect();
    let no_row: Vec<&str> = extracted.iter().map(String::as_str).filter(|c| !row_ids.contains(c)).collect();
    let join = |v: &[&str]| if v.is_empty() { "(none)".to_string() } else { v.join(", ") };
    let mut h = String::new();
    let _ = writeln!(h, "== CRATE MEMBERSHIP (backend/src -> target crate; generated from bands.rs) ==");
    let _ = writeln!(h, "  Line counts are newline counts (wc -l). Every row counts the backend/src files");
    let _ = writeln!(h, "  crate_of_backend_mod maps to that crate. An extracted crate keeps a row here only");
    let _ = writeln!(h, "  if it left permanent facade stubs in backend/src: the row then measures that facade");
    let _ = writeln!(h, "  surface the backend still composes, not work left to carve. An extracted crate that");
    let _ = writeln!(h, "  left no stub has no row. So TOTAL does not fall to zero as carving completes — it");
    let _ = writeln!(h, "  converges to the summed line count of every facade stub that remains.");
    let _ = writeln!(h, "  Extracted, keeps a facade row: {}.", join(&with_row));
    let _ = writeln!(h, "  Extracted, left no stub (no row): {}.", join(&no_row));
    let _ = writeln!(h, "  TEST-SCAFFOLD collects cfg(test)-only backend-local modules that ship in no crate");
    let _ = writeln!(h, "  and are never carved. UNASSIGNED collects any top-level module crate_of_backend_mod");
    let _ = writeln!(h, "  does not name — a finding to resolve in bands.rs, not a --check failure; it is empty");
    let _ = writeln!(h, "  when every module is placed.");
    h
}

fn write_membership(s: &mut String, m: &Membership, extracted: &BTreeSet<String>) {
    s.push_str(&membership_header(m, extracted));
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
    extracted: &BTreeSet<String>,
    unbanded: &BTreeSet<String>,
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

    write_membership(&mut s, membership, extracted);

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

    // Carved-but-unbanded backstop: a crates/<dir> whose name is a real band but
    // whose floor_crate_of_dir arm was never added (see `carved_but_unbanded`).
    // Printed always so its absence is itself the healthy state; a row here is a
    // bands.rs omission, not a --check failure by itself (though it silently
    // undercounts the ratchet, which is why it is surfaced loudly).
    let _ = writeln!(
        s,
        "== CARVED BUT UNBANDED (crates/<dir> names a real band but floor_crate_of_dir maps it to floor:<dir>) — {} ==",
        unbanded.len()
    );
    if unbanded.is_empty() {
        let _ = writeln!(s, "  none — every extracted crates/<dir> that names a real band has its bands.rs arm.");
    } else {
        for name in unbanded {
            let _ = writeln!(
                s,
                "  {name}: band {} but scores as floor:{name} — add a floor_crate_of_dir(\"{name}\") arm in bands.rs.",
                band_of(name)
            );
        }
    }
    let _ = writeln!(s);

    // Carve readiness + Tarjan on the target-crate graph.
    write_carve_analysis(&mut s, records, extracted);

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

/// A carve-target crate's state at the current extraction front.
#[derive(Debug, PartialEq, Eq)]
enum CarveState {
    /// Already carved into `crates/*`; it has left the frontier.
    Extracted,
    /// Has illegal outgoing edges (the count) that must reach zero first.
    Blocked(usize),
    /// No illegal outgoing and every carve-target dep is extracted — carve next.
    Ready,
    /// No illegal outgoing but a carve-target dep is not yet extracted.
    Waiting,
}

/// Classify a carve-target crate. `EXTRACTED` wins over everything: a crate already
/// carved is done, and reading it as `READY` is the exact defect this pins — the
/// frontier pointing at finished work. Otherwise an illegal outgoing edge blocks the
/// carve; failing that, the crate is READY once every carve-target dep is extracted
/// and WAITING until then. `carve_target_deps` is the crate's legal deps that are
/// themselves carve targets (deps on the floor or an already-extracted crate never
/// block), so with `extracted` empty this reproduces the pre-extraction reading.
fn carve_state(
    crate_id: &str,
    illegal_out: usize,
    carve_target_deps: &[String],
    extracted: &BTreeSet<String>,
) -> CarveState {
    if extracted.contains(crate_id) {
        return CarveState::Extracted;
    }
    if illegal_out > 0 {
        return CarveState::Blocked(illegal_out);
    }
    if carve_target_deps.iter().all(|d| extracted.contains(d)) {
        CarveState::Ready
    } else {
        CarveState::Waiting
    }
}

/// Per-crate carve readiness (illegal outgoing edges block a carve) and a Tarjan
/// SCC pass on the production target-crate graph.
fn write_carve_analysis(s: &mut String, records: &[RefRecord], extracted: &BTreeSet<String>) {
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

    let extracted_here: Vec<&str> =
        CARVE_ORDER.iter().map(|(_, c)| *c).filter(|c| extracted.contains(*c)).collect();
    let extracted_line = if extracted_here.is_empty() {
        "No carve-target crate is extracted yet.".to_string()
    } else {
        format!("Extracted carve targets so far: {}.", extracted_here.join(", "))
    };
    let _ = writeln!(s, "== CARVE READINESS (frontier — legal-dependency order, lowest band first) ==");
    let _ = writeln!(s, "  READY = no illegal outgoing edge AND every crate it depends on is already");
    let _ = writeln!(s, "  extracted. EXTRACTED = already carved into crates/*, detected from this run's own");
    let _ = writeln!(s, "  scan (a crates/<dir> that resolves to a real band, not floor:<dir>) — it has left");
    let _ = writeln!(s, "  the frontier and is shown for context, not as work. {extracted_line}");
    let _ = writeln!(s, "  A crate with no illegal outgoing whose carve-target deps are all extracted reads");
    let _ = writeln!(s, "  READY; one still waiting on an un-extracted dep reads WAITING. Re-run after each");
    let _ = writeln!(s, "  carve; the frontier is the first crate below that is READY.");
    let frontier =
        order.iter().find(|c| !extracted.contains(c.as_str())).cloned().unwrap_or_default();
    let frontier_illegal: usize = blockers.get(frontier.as_str()).map_or(0, |e| e.values().sum());
    if frontier_illegal == 0 {
        let _ = writeln!(s, "  FRONTIER -> {frontier}: READY now (no illegal outgoing; every crate it depends on is extracted).");
    } else {
        let _ = writeln!(s, "  FRONTIER -> {frontier}: BLOCKED on {frontier_illegal} illegal outgoing. No crate is carvable until it clears.");
    }
    let _ = writeln!(s);
    let empty: BTreeSet<String> = BTreeSet::new();
    for crate_id in &order {
        // The crate's legal deps that are themselves carve targets (a dep on the
        // floor or an already-extracted crate never blocks). Shown in full so the
        // dependency structure is stable across carves; only readiness moves.
        let deps = dep_pending(crate_id, &empty);
        let dep_str = if deps.is_empty() {
            "floor / adapter-model only".to_string()
        } else {
            deps.join(", ")
        };
        let illegal = blockers.get(crate_id.as_str());
        let illegal_total: usize = illegal.map_or(0, |e| e.values().sum());
        let state = match carve_state(crate_id, illegal_total, &deps, extracted) {
            CarveState::Extracted => "EXTRACTED".to_string(),
            CarveState::Blocked(n) => {
                let targets: Vec<String> =
                    illegal.map(|e| e.iter().map(|(t, c)| format!("{t}(x{c})")).collect()).unwrap_or_default();
                format!("BLOCKED — {n} illegal outgoing to {}", targets.join(", "))
            }
            CarveState::Ready => "READY".to_string(),
            CarveState::Waiting => {
                "WAITING — no illegal outgoing, but its deps are not extracted yet".to_string()
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(lines: &[&str]) -> BTreeSet<String> {
        lines.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn membership_header_lists_each_extracted_crate_under_its_facade_state() {
        // The defect this pins (stale-generated-prose class): the header claimed "a
        // crate already extracted (core, adapter-model) ... has no row here", but
        // identity and workspaces are extracted AND each keep a facade-stub row. A
        // reader trusting the header reads that facade surface as uncarved work and
        // expects TOTAL -> 0; it converges to the summed facade stubs instead.
        //
        // The assertion is STRUCTURAL, not a substring match over the whole header:
        // it parses the two facade-state lines and checks exact membership on each.
        // A whole-header `contains(id)` would pass on the single most likely bug in
        // membership_header — the two filters swapped, which puts every crate on the
        // WRONG line — because the id still appears somewhere. That is the same
        // substring-for-structural trap the plan flagged at #386; this test must not
        // reproduce it. So it asserts both directions: a crate with a row is on the
        // keeps line and NOT the no-stub line, and a crate with no row vice versa.
        let row = |id: &str, files: usize| MemberRow {
            crate_id: id.to_string(),
            modules: if files == 0 { BTreeSet::new() } else { edges(&["x.rs"]) },
            lines: files * 10,
            files,
        };
        // workspaces: extracted, kept a facade row (real files). server: uncarved.
        // tasks: extracted but left NO facade stub, yet build_membership still emits
        // a zero-file placeholder row for it (it is a CARVE_ORDER crate) — this is
        // the case row presence alone gets wrong. core: extracted, no row at all.
        let m = Membership {
            rows: vec![row("workspaces", 5), row("server", 12), row("tasks", 0)],
        };
        let extracted = edges(&["workspaces", "core", "tasks"]);
        let header = membership_header(&m, &extracted);

        // The comma-separated crate list printed after a header line's colon, as
        // exact tokens — not a substring haystack.
        let names_on = |needle: &str| -> Vec<String> {
            let line = header
                .lines()
                .find(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("header is missing the `{needle}` line:\n{header}"));
            let Some((_, list)) = line.rsplit_once(": ") else {
                panic!("header line has no `label: list` shape: {line}");
            };
            list.trim_end_matches('.').split(", ").map(str::to_string).collect()
        };
        // Needles unique to the two enumeration lines: the bare phrases "keeps a
        // facade row" / "left no stub" also occur in the body prose above, so match
        // the "Extracted, " prefix that only the enumeration lines carry.
        let keeps = names_on("Extracted, keeps a facade row");
        let no_stub = names_on("Extracted, left no stub");
        let has = |v: &[String], id: &str| v.iter().any(|c| c == id);

        // workspaces is extracted with a real (files > 0) row -> keeps line only.
        assert!(has(&keeps, "workspaces"), "extracted crate with a facade row must be on the keeps-a-facade-row line: {keeps:?}");
        assert!(!has(&no_stub, "workspaces"), "extracted crate with a facade row must NOT be on the no-stub line: {no_stub:?}");
        // core is extracted with no row at all -> no-stub line only.
        assert!(has(&no_stub, "core"), "extracted crate with no row must be on the no-stub line: {no_stub:?}");
        assert!(!has(&keeps, "core"), "extracted crate with no row must NOT be on the keeps-a-facade-row line: {keeps:?}");
        // tasks is extracted with a ZERO-FILE placeholder row -> it left no facade
        // stub, so it belongs on the no-stub line. Row presence alone would wrongly
        // place it on the keeps line; `files > 0` is what makes this correct.
        assert!(has(&no_stub, "tasks"), "extracted crate with a zero-file placeholder row must be on the no-stub line: {no_stub:?}");
        assert!(!has(&keeps, "tasks"), "extracted crate with a zero-file placeholder row must NOT be on the keeps-a-facade-row line: {keeps:?}");
    }

    #[test]
    fn an_extracted_crate_reads_extracted_not_ready() {
        // The defect this pins: the readiness table pointing at finished work.
        // identity is carved, so it must read EXTRACTED, never READY.
        assert_eq!(carve_state("identity", 0, &[], &edges(&["identity"])), CarveState::Extracted);
        // The base-case READY only holds *before* the carve, when nothing is extracted.
        assert_eq!(carve_state("identity", 0, &[], &edges(&[])), CarveState::Ready);
    }

    #[test]
    fn a_dep_becoming_extracted_flips_a_waiter_to_ready() {
        let deps = vec!["identity".to_string()];
        // workspaces waits while identity is still inside the backend...
        assert_eq!(carve_state("workspaces", 0, &deps, &edges(&[])), CarveState::Waiting);
        // ...and flips to READY the instant identity is extracted. This flip is the
        // whole deliverable of the identity carve, and the table must show it.
        assert_eq!(carve_state("workspaces", 0, &deps, &edges(&["identity"])), CarveState::Ready);
    }

    #[test]
    fn illegal_outgoing_blocks_regardless_of_extraction() {
        let deps = vec!["identity".to_string(), "workspaces".to_string()];
        // An illegal edge blocks even once a dep is extracted — extraction never
        // masks the ratchet, and the count is carried through verbatim.
        assert_eq!(
            carve_state("tasks", 57, &deps, &edges(&["identity"])),
            CarveState::Blocked(57)
        );
    }

    #[test]
    fn extraction_signal_is_the_band_not_a_hardcoded_list() {
        // Derived from the scan: a Floor-scope file whose source crate resolved to a
        // real band (floor_crate_of_dir returned the band id, not floor:<dir>) is
        // extracted; a floor:<dir> source is not. Pins the signal to the band
        // distinction, so adding the bands arm at carve time is what flips a crate to
        // extracted — no hand-maintained "identity is done" line.
        let df = |source: &str| DiscoveredFile {
            abs: PathBuf::new(),
            rel: String::new(),
            base_mod: Vec::new(),
            scope: ScopeKind::Floor,
            source_crate: source.to_string(),
        };
        let files = vec![
            df(&floor_crate_of_dir("identity")), // "identity" — a real band-3 crate
            df(&floor_crate_of_dir("events")),   // "floor:events" — the floor
        ];
        let extracted = extracted_crates(&files);
        assert!(extracted.contains("identity"), "a real-band crates/* dir is extracted");
        assert!(!extracted.contains("floor:events"), "a floor:<dir> source is not extracted");
        assert!(
            !extracted.iter().any(|c| c.starts_with("floor:")),
            "no floor id ever counts as an extracted crate"
        );
    }

    #[test]
    fn carved_but_unbanded_fires_on_a_target_band_dir_that_still_scores_as_floor() {
        let df = |source: &str| DiscoveredFile {
            abs: PathBuf::new(),
            rel: String::new(),
            base_mod: Vec::new(),
            scope: ScopeKind::Floor,
            source_crate: source.to_string(),
        };
        // Synthetic omission tracking the current carve frontier: `sandbox-k8s`
        // (band 7) is the next crate to be extracted and has no floor_crate_of_dir
        // arm yet, so its dir resolves to floor:sandbox-k8s. The guard MUST fire and
        // name it. Constructed via floor_crate_of_dir so the day PR-4 adds the arm,
        // source_crate becomes "sandbox-k8s", this stops being a floor: id, and the
        // test breaks loudly — repoint it to the next unbanded frontier
        // (sandbox-docker) rather than asserting a stale string. `tasks` then
        // `sandbox-control` were this example until their arms landed; the break is
        // the forcing function that keeps the guard pointed at a real, current gap.
        let missing = floor_crate_of_dir("sandbox-k8s");
        assert_eq!(
            missing, "floor:sandbox-k8s",
            "the omission this test simulates: no bands arm yet"
        );
        let files = vec![
            df(&missing),                        // floor:sandbox-k8s — band 7, unbanded: FIRES
            df(&floor_crate_of_dir("events")),   // floor:events — a real floor crate
            df(&floor_crate_of_dir("identity")), // "identity" — properly banded, extracted
        ];
        let unbanded = carved_but_unbanded(&files);
        assert!(
            unbanded.contains("sandbox-k8s"),
            "a target-band dir scoring as floor must be surfaced"
        );
        assert!(!unbanded.contains("events"), "a genuine floor crate (band 50) must not fire");
        assert!(
            !unbanded.iter().any(|n| n == "identity"),
            "a properly-banded extracted crate is not floor:-prefixed, so it never reaches the guard"
        );
        assert_eq!(unbanded.len(), 1, "exactly the one omission, no false positives");
    }

    #[test]
    fn carved_but_unbanded_is_empty_on_todays_tree_shape() {
        // The floor crates that exist today all name bands outside 1..=10 (band 50),
        // so the guard is silent until a real target-band crate is carved without its
        // arm. This pins the zero-false-positive property.
        let df = |source: &str| DiscoveredFile {
            abs: PathBuf::new(),
            rel: String::new(),
            base_mod: Vec::new(),
            scope: ScopeKind::Floor,
            source_crate: source.to_string(),
        };
        let floors = ["events", "backend-core", "api-schemas", "sandbox-proto", "cli", "api-client"];
        let files: Vec<_> = floors.iter().map(|d| df(&floor_crate_of_dir(d))).collect();
        assert!(carved_but_unbanded(&files).is_empty(), "no real floor crate names a target band");
    }

    #[test]
    fn parse_ceiling_reads_the_count_line_through_surrounding_prose() {
        let content = "\
# Illegal production Cargo edges: some prose.
# more prose about the ratchet.
# Illegal edge count: 179
a -> b [CODE] x.rs:1  crate::b::T
";
        assert_eq!(parse_ceiling(content), Some(179));
    }

    #[test]
    fn parse_ceiling_is_none_when_the_line_is_absent() {
        assert_eq!(parse_ceiling("# no count here\nsome edge\n"), None);
        assert_eq!(parse_ceiling(""), None);
    }

    #[test]
    fn parse_illegal_edges_takes_only_edge_lines_above_the_intra_doc_section() {
        let content = "\
# header comment
# Illegal edge count: 2
a -> b [CODE] x.rs:1  crate::b::T
c -> d [ATTR] y.rs:2  crate::d::U

# --- intra-doc paths pointing up (NOT Cargo edges) ---
# Doc-path count (not in the illegal count above): 1
e -> f [DOC] z.rs:3  crate::f::V
";
        let got = parse_illegal_edges(content);
        assert_eq!(
            got,
            edges(&[
                "a -> b [CODE] x.rs:1  crate::b::T",
                "c -> d [ATTR] y.rs:2  crate::d::U",
            ]),
            "comments, blanks, and the intra-doc section are all excluded"
        );
    }

    #[test]
    fn within_the_ceiling_is_not_a_violation() {
        // Regenerating a carve that removed edges: current below the committed
        // ceiling is fine — the ratchet only asserts an upper bound.
        assert_eq!(ceiling_violation(147, 179, &edges(&[]), &edges(&[])), None);
        assert_eq!(ceiling_violation(179, 179, &edges(&[]), &edges(&[])), None);
    }

    #[test]
    fn an_added_edge_names_itself_when_the_baseline_is_not_yet_regenerated() {
        // The author added an edge but did not regenerate, so the committed list
        // still holds the old set — the new edge can be named exactly.
        let committed = edges(&["a -> b [CODE] x.rs:1  crate::b::T"]);
        let current = edges(&[
            "a -> b [CODE] x.rs:1  crate::b::T",
            "c -> server [CODE] new.rs:9  crate::credential_sync::CredentialSync",
        ]);
        let Some(msg) = ceiling_violation(2, 1, &current, &committed) else {
            panic!("count 2 over ceiling 1 must be a violation");
        };
        assert!(msg.contains("rose to 2, above the ratcheted ceiling of 1"));
        assert!(
            msg.contains("1 new illegal edge(s) not in the committed baseline")
                && msg.contains("crate::credential_sync::CredentialSync"),
            "the added edge is named: {msg}"
        );
        assert!(
            msg.contains("Regenerating will NOT fix this"),
            "the message pre-empts the reflexive regenerate-and-commit: {msg}"
        );
    }

    #[test]
    fn a_regenerated_baseline_over_ceiling_points_at_the_diff() {
        // The reflexive "just regenerate and commit": now the committed list
        // already contains every current edge, so the excess cannot be named
        // from files alone — the message must hand off to `git diff`, and still
        // refuse to accept the raised count.
        let all = edges(&[
            "a -> b [CODE] x.rs:1  crate::b::T",
            "c -> server [CODE] new.rs:9  crate::credential_sync::CredentialSync",
        ]);
        let Some(msg) = ceiling_violation(2, 1, &all, &all) else {
            panic!("a regenerated-but-raised count must still be a violation");
        };
        assert!(
            msg.contains("cannot be named from files alone")
                && msg.contains("git diff origin/main"),
            "with no new edge to name it points at the diff: {msg}"
        );
        assert!(msg.contains("Regenerating will NOT fix this"));
    }

    #[test]
    fn build_baseline_header_is_the_passed_ceiling_not_the_edge_count() {
        // The heart of the asymmetry: the header carries the ceiling the caller
        // computed (`min(committed, current)`), even when more edges are listed
        // than the ceiling. A regeneration that added an edge writes a header
        // that sits BELOW its own edge list — exactly the state `--check` fails.
        let illegal = edges(&[
            "a -> b [CODE] x.rs:1  crate::b::T",
            "c -> d [CODE] y.rs:2  crate::d::U",
            "e -> f [CODE] z.rs:3  crate::f::V",
        ]);
        let out = build_baseline(&illegal, &edges(&[]), 2);
        assert!(
            out.contains("# Illegal edge count: 2"),
            "header is the ceiling (2), not illegal.len() (3): {out}"
        );
        // All three edges are still listed — the header is a ceiling, not a tally.
        assert!(out.contains("crate::f::V"));
    }
}

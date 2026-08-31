#!/usr/bin/env bash
#
# The seam is the only way across a crate boundary.
#
# Three invariants, all about what one crate is allowed to know about another:
#
#   1. **A store stays private to its crate.** A `*Dao` — the trait, its `Pg*`
#      implementation, or a mock of it — belongs to the crate that owns the
#      table. A peer crate names the narrow port over it instead. Each pair that
#      is allowed to cross is listed below with the reason it is allowed; a pair
#      that is not listed fails, naming the file and line.
#
#   2. **A crate that owns no table declares no database driver.** `sqlx` under
#      `[dependencies]` means the crate compiles a Postgres driver into every
#      build that names it. Only a crate that owns a table, or the composition
#      root that wires the pool, has a reason to. `optional = true` does not
#      count: the driver is behind a feature, which is the point.
#
#   3. **The Adapter facade re-exports nothing that belongs to another crate.**
#      `crates/loops/src/adapters/` is where every provider looks, which is why
#      a `pub use` of a lower crate's item there is expensive: it gives one type
#      two paths, and the longer one wins by sitting closer to the caller. The
#      rule is stated in crates/loops/AGENTS.md — "one path to a type is the
#      point". A re-export that is on its way out is listed below with the
#      condition that removes it, the way a `*Dao` pair is listed with its
#      reason; the list only shrinks.
#
#      This invariant carries a self-test. A re-export ban that stops matching
#      is indistinguishable from a facade with nothing to report, so the scan
#      runs against a synthetic fixture written to trip it, on every invocation,
#      and fails if it does not fire. An inert gate is the one change a ratchet
#      cannot protect.
#
# All are grep-level checks, not a Rust parser, but the source scan tokenizes
# enough that a reference cannot hide in a comment, a string, or a test module.
# `crates/backend` is the composition root and is exempt from (1) for the same
# reason `check-dao-composition-root.sh` exempts `compose.rs`: assembling every
# domain is what it is for.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ── Pairs a written rule already settles ─────────────────────────────────────
# `<consumer-crate> <dao path>`; `*` matches any consumer. Each entry names the
# document that states the relationship. A `*Dao` reached only from test code is
# not listed here — the scan drops test context before matching.
read -r -d '' ALLOWED_DAOS <<'PAIRS' || true
# The Adapter kernel's connection store. All seven methods are used by every
# consumer, and every consumer is inside the Adapter context that owns the
# table. Stated in crates/loops/AGENTS.md.
*                 moatless_adapter_model::dao::AdapterConnectionDao

# An Adapter that upserts Bindings as well as reading and writing them drives
# the Binding lifecycle, so it names the store rather than a second read
# surface beside it. Stated in crates/loops/AGENTS.md.
adapter-github-pr moatless_loops::bindings::dao::TaskBindingDao
adapter-linear    moatless_loops::bindings::dao::TaskBindingDao

# `pairing.rs` alone, whose deletion is a behaviour question rather than a
# cleanup. Recorded in docs/deferred-changes.md.
adapter-telegram  moatless_loops::loops::dao::LoopDao

# File storage is an infrastructure band below every consumer, so the edge is
# already downward and a port over it would narrow nothing.
*                 moatless_core::file_storage::dao::FileDao

# The workspace credential service owns the provider-token lifecycle — it
# writes the rows the identity band reads — so it names the store it drives.
workspaces        moatless_identity::auth::provider_token_dao::ProviderTokenDao

# A single-method write surface: every consumer appends one audit row and reads
# none, so it is already as narrow as a port over it would be. Stated in
# crates/identity/AGENTS.md.
workspaces        moatless_identity::secret::audit::SecretAuditDao
PAIRS

# ── Crates with a reason to compile a database driver ────────────────────────
# A crate owning a table (see check-table-ownership.sh), plus the composition
# root that opens the pool.
TABLE_OWNERS="adapter-telegram backend-core core identity loops sandbox-control tasks workspaces t3code backend"

# The source scan, hoisted so `xargs` never has to carry it as an argument
# pattern — the program itself contains `{}`.
read -r -d '' SCAN <<'PERL' || true

      my $src = $_;
      # Blank strings and comments, preserving newlines so line numbers hold.
      $src =~ s{
          r(\#*)".*?"\g{1}
        | b?"(?:\\.|[^"\\])*"
        | //[^\n]*
        | /\*.*?\*/
      }{ " " . ("\n" x (($& =~ tr/\n//))) }gexs;
      # Drop `#[cfg(<test>)] mod NAME { ... }` blocks; a test naming a concrete
      # store against a real pool is what an integration test is.
      $src =~ s{^[ \t]*\#\[cfg\([^\n]*test[^\n]*\][ \t]*\n[ \t]*(?:pub(?:\([a-z]+\))?[ \t]+)?mod[ \t]+\w+[ \t]*\{.*?\n\}}
               { "\n" x (($&) =~ tr/\n//) }gmse;
      # Flatten `a::b::{X, Y as Z}` onto one line each, so a braced `use`
      # group cannot hide a store the single-path form would report.
      $src =~ s{(moatless_[a-z_]+(?:::[a-z_]+)*)::\{([^{}]*)\}}{
        my ($stem, $items) = ($1, $2);
        join ", ", map { my $i = $_; $i =~ s/\s+as\s+\w+//; $i =~ s/^\s+|\s+$//g;
                         $i =~ /^[A-Za-z_][\w:]*$/ ? "$stem\::$i" : $i }
                   split /,/, $items;
      }gse;
      my $n = 0;
      for my $l (split /\n/, $src, -1) {
        $n++;
        while ($l =~ /(moatless_[a-z_]+(?:::[a-z_]+)*::[A-Za-z]*Dao)\b/g) {
          print "$ARGV:$n:$1\n";
        }
      }
PERL

# ── Facade re-exports on their way out ───────────────────────────────────────
# `<file-relative-to-crates/loops/src/adapters> <re-exported crate>`. Each entry
# names the change that deletes it; the list only shrinks. An entry with no
# condition is a second path nobody chose, not an exception.
read -r -d '' ALLOWED_REEXPORTS <<'REEXPORTS' || true
# Empty, and it got here by shrinking rather than by never having been used.
# The one entry was `error.rs moatless_adapter_model`: `SharedAdapterError` and
# `ProviderDecodeError` were named from the kernel through this facade so the
# provider error paths spelled one `error::` prefix while `AdapterError` still
# lived here. The Adapter contract move — `AdapterError`, its renderer, and the
# capability traits into `moatless-adapter-model` — removed the module doing the
# forwarding, so the entry cleared by construction rather than by exception.
# Nothing may be added back without the condition that deletes it again.
REEXPORTS

# The re-export scan, hoisted so the real facade and the self-test fixture run
# the same code. A `pub use` names its crate on the same line even when the
# braced group that follows spans several lines, so a line match sees every form.
read -r -d '' REEXPORT_SCAN <<'PERL' || true

      my $src = $_;
      # Blank strings and comments, preserving newlines so line numbers hold.
      $src =~ s{
          r(\#*)".*?"\g{1}
        | b?"(?:\\.|[^"\\])*"
        | //[^\n]*
        | /\*.*?\*/
      }{ " " . ("\n" x (($& =~ tr/\n//))) }gexs;
      my $n = 0;
      for my $l (split /\n/, $src, -1) {
        $n++;
        print "$ARGV:$n:$1\n"
          if $l =~ /^\s*pub\s+use\s+(moatless_[a-z_]+)\b/;
      }
PERL

scan_reexports() {
  find "$1" -name '*.rs' -print0 2>/dev/null \
    | xargs -0 -r perl -0777 -ne "$REEXPORT_SCAN"
}

fail=0

# ── 1. Cross-crate `*Dao` references ─────────────────────────────────────────
for dir in crates/*/; do
  crate="$(basename "$dir")"
  [ "$crate" = "backend" ] && continue
  [ -d "$dir/src" ] || continue

  while IFS= read -r hit; do
    file="${hit%%:*}"
    rest="${hit#*:}"
    line="${rest%%:*}"
    path="${rest#*:}"

    allowed=0
    while read -r consumer dao; do
      case "$consumer" in ''|'#'*) continue ;; esac
      [ "$dao" = "$path" ] || continue
      { [ "$consumer" = '*' ] || [ "$consumer" = "$crate" ]; } && allowed=1 && break
    done <<< "$ALLOWED_DAOS"

    if [ "$allowed" -eq 0 ]; then
      echo "FAIL $file:$line — $crate names $path, which is another crate's store"
      fail=1
    fi
  done < <(find "${dir%/}/src" -name '*.rs' -print0 | xargs -0 perl -0777 -ne "$SCAN")
done

# ── 2. `sqlx` under `[dependencies]` for a crate that owns no table ──────────
for manifest in crates/*/Cargo.toml; do
  crate="$(basename "$(dirname "$manifest")")"
  case " $TABLE_OWNERS " in *" $crate "*) continue ;; esac

  hard="$(awk '
    /^\[dependencies\]/ { in_deps = 1; next }
    /^\[/               { in_deps = 0 }
    in_deps && /^sqlx[[:space:]]*=/ && !/optional[[:space:]]*=[[:space:]]*true/ { print NR }
  ' "$manifest")"

  if [ -n "$hard" ]; then
    echo "FAIL $manifest:$hard — $crate owns no table and declares sqlx under [dependencies]"
    fail=1
  fi
done

# ── 3. Re-exports published by the Adapter facade ────────────────────────────
FACADE="crates/loops/src/adapters"

while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  file="${hit%%:*}"
  rest="${hit#*:}"
  line="${rest%%:*}"
  reexported="${rest#*:}"
  rel="${file#"$FACADE"/}"

  allowed=0
  while read -r afile acrate; do
    case "$afile" in ''|'#'*) continue ;; esac
    [ "$afile" = "$rel" ] && [ "$acrate" = "$reexported" ] && allowed=1 && break
  done <<< "$ALLOWED_REEXPORTS"

  if [ "$allowed" -eq 0 ]; then
    echo "FAIL $file:$line — the Adapter facade re-exports $reexported, giving that"
    echo "     item a second path. Name the owning crate directly at each use site."
    fail=1
  fi
done < <(scan_reexports "$FACADE")

# The self-test. A ban that stopped matching is indistinguishable from a facade
# with nothing to report, so prove the scan fires before trusting its silence.
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
printf 'pub use moatless_adapter_model::error::SharedAdapterError;\n' > "$fixture/facade.rs"
if [ -z "$(scan_reexports "$fixture")" ]; then
  echo "FAIL scripts/check-crate-seams.sh — the re-export scan found nothing in a"
  echo "     fixture written to trip it. The check is inert; repair it before"
  echo "     trusting any run that reported this invariant clean."
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo
  echo "A crate reached past a published seam."
  echo
  echo "Either use the port the owning crate publishes, or — if the edge is"
  echo "deliberate — add the pair to ALLOWED_DAOS (a store) or ALLOWED_REEXPORTS"
  echo "(a facade re-export) above with the reason, and state that reason in the"
  echo "owning crate's AGENTS.md. The list is the documentation; an entry without"
  echo "a reason is a bypass."
  exit 1
fi

echo "OK: no crate names another crate's store outside the documented set, no"
echo "crate that owns a table-free surface declares sqlx, and the Adapter facade"
echo "re-exports nothing outside the shrinking set."

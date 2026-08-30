#!/usr/bin/env bash
#
# The seam is the only way across a crate boundary.
#
# Two invariants, both about what one crate is allowed to know about another:
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
# Both are grep-level checks, not a Rust parser, but the source scan tokenizes
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

if [ "$fail" -ne 0 ]; then
  echo
  echo "A crate reached past a published seam."
  echo
  echo "Either use the port the owning crate publishes, or — if the edge is"
  echo "deliberate — add the pair to ALLOWED_DAOS above with the reason, and"
  echo "state that reason in the owning crate's AGENTS.md. The list is the"
  echo "documentation; an entry without a reason is a bypass."
  exit 1
fi

echo "OK: no crate names another crate's store outside the documented set, and no"
echo "crate that owns a table-free surface declares sqlx."

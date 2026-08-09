#!/usr/bin/env bash
#
# Step 3 invariant — the port is the only way to the table.
#
# The composition root (crates/backend/src/compose.rs) is the single place a concrete
# `Pg*Dao` is constructed; every other production site takes an `Arc<dyn *Dao>`
# port instead. A `Pg*Dao(pool)` / `Pg*Dao::new(pool)` construction anywhere
# else is a bypass of the port abstraction, and this check fails on it, naming
# the file and line.
#
# Exclusions, each deliberate and stated (not a pattern narrowed until green):
#
#   - crates/backend/src/**/dao.rs      the DAO *definitions* live here: the trait, the
#                                `Pg*Dao` struct, and its inherent impls.
#   - crates/backend/src/compose.rs     the composition root itself.
#   - crates/backend/src/secret/cli.rs  a second entry path in the same binary (there is
#                                no `[[bin]]`): the `secrets` subcommand is
#                                dispatched at main.rs:50, before the server
#                                runtime and its composed `Ports` exist. It builds
#                                its own pool and exactly the two DAOs that one
#                                command needs (`PgSecretDao`, `PgSecretAuditDao`).
#                                It runs before the port wiring, so it cannot take
#                                a port from it — not a bypass of that wiring.
#   - #[cfg(test)] / #[cfg(all(test, ...))] modules, and module files that are
#                                declared only under such a gate: test code
#                                constructing a `Pg*Dao` against a real pool is
#                                what an integration test *is*. The invariant
#                                protects production wiring, not test setup.
#   - crates/backend/tests/**           the same reason; integration tests are not
#                                scanned here at all.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SRC="crates/backend/src"

# ── 1. Module files that are test-only ───────────────────────────────────────
# A file whose module is declared `#[cfg(<test>)] mod NAME;` in its parent is
# reachable only under cfg(test), so it is test code even without an internal
# cfg attribute (e.g. sandbox/lifecycle/contract_tests.rs).
declare -A TEST_ONLY_FILE
while IFS= read -r parent; do
  dir="$(dirname "$parent")"
  while IFS= read -r name; do
    for cand in "$dir/$name.rs" "$dir/$name/mod.rs"; do
      [ -f "$cand" ] && TEST_ONLY_FILE["$cand"]=1
    done
  done < <(
    awk '
      prev ~ /#\[cfg\((all\()?test/ &&
      $0 ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+[[:space:]]*;/ {
        name = $0
        sub(/.*mod[[:space:]]+/, "", name)
        sub(/[[:space:]]*;.*/, "", name)
        print name
      }
      { prev = $0 }
    ' "$parent"
  )
done < <(grep -rlE '#\[cfg\((all\()?test' "$SRC" --include='*.rs')

# ── 2. Scan every source file ────────────────────────────────────────────────
# This is a syntactic check, not a Rust parser, but it tokenizes enough that a
# construction cannot hide in comments, strings, or whitespace. A per-file perl
# pass, in order:
#
#   1. Blanks every string literal (normal `"…"` with escapes, raw `r#"…"#`,
#      byte/char literals) and every `//` and `/* */` comment, replacing each
#      with as many newlines as it spanned so reported line numbers stay exact.
#      Blanking strings first is what makes a `//` inside a URL literal
#      (`build("https://x", PgFooDao::new(p))`) stay code, not a comment — the
#      earlier line-comment strip erased the real construction after such a URL,
#      a silent pass. Blanking (not preserving) string contents also means a
#      `"PgFooDao(p)"` written as data never false-matches.
#   2. Drops each top-level `#[cfg(<test>)] mod NAME { ... }` block (test setup
#      building a `Pg*Dao` against a real pool is what an integration test is;
#      the module-file exclusion in step 1 covers test-only files).
#   3. Matches a `Pg*Dao` construction on the surviving code, in two shapes:
#        - `PgFooDao(…)` / `PgFooDao::new(…)`, with any whitespace, newlines, or
#          (now-blanked) comment between the path, an optional `::new`, and `(` —
#          so `PgFooDao\n  ::new(p)` and `PgFooDao /* c */ (p)` both catch.
#        - UFCS `<PgFooDao>::new(…)` and `<PgFooDao as Trait>::new(…)`.
#      A tuple-struct definition (`struct PgFooDao(`) is excluded; a trait impl
#      header names no `(` after the type, so it never matches.
#
# The one Rust spelling this cannot catch is an aliased import that renames the
# type away from its `Pg*Dao` spelling at the call site
# (`use …::PgFooDao as Foo; Foo(p)`): the call site holds no `Pg` token, so a
# grep cannot see it — closing it needs type resolution (a clippy lint over
# resolved paths), which is out of scope for a CI grep. The invariant this script
# enforces is therefore precisely: no construction that *names* a `Pg*Dao` path
# (directly, via `::new`, via UFCS, or through any comment/whitespace) exists
# outside the composition root. That is the whole population in-tree today; a
# rename-to-hide is adversarial, not an accident this backstop is sized for.
violations=0
while IFS= read -r file; do
  case "$file" in
    */dao.rs) continue ;;
    "$SRC/compose.rs") continue ;;
    "$SRC/secret/cli.rs") continue ;;
  esac
  [ -n "${TEST_ONLY_FILE[$file]:-}" ] && continue

  matches="$(
    perl -0777 -ne '
      my $src = $_;
      # Blank strings (normal, raw, byte/char) and comments, preserving newlines.
      $src =~ s{
          r(\#*)".*?"\g{1}                 # raw string r"…" / r#"…"#
        | b?"(?:\\.|[^"\\])*"              # normal / byte string, escapes honoured
        | b?'"'"'(?:\\.|[^'"'"'\\])'"'"'   # char / byte-char literal
        | //[^\n]*                          # line comment
        | /\*.*?\*/                         # block comment
      }{ " " . ("\n" x (($& =~ tr/\n//))) }gexs;
      # Drop top-level `#[cfg(<test>)] mod NAME { ... }` blocks (close is the
      # first column-0 `}`; nested closes are indented), preserving line count.
      $src =~ s{^[ \t]*\#\[cfg\([^\n]*test[^\n]*\][ \t]*\n[ \t]*(?:pub(?:\([a-z]+\))?[ \t]+)?mod[ \t]+\w+[ \t]*\{.*?\n\}}
               { "\n" x (($&) =~ tr/\n//) }gmse;
      my %seen;
      # Direct and `::new` forms.
      while ($src =~ /(?<![A-Za-z0-9_])(Pg[A-Za-z]+Dao)\s*(?:::\s*new\s*)?\(/g) {
        my $pre = substr($src, 0, $-[1]);
        next if $pre =~ /\bstruct\s+$/;   # a declaration is not a construction
        next if $pre =~ /<\s*$/;          # the `<PgFooDao>` UFCS form, caught below
        my $lineno = 1 + ($pre =~ tr/\n//);
        print "$ARGV:$lineno: $1(...)\n" unless $seen{$lineno}++;
      }
      # UFCS: `<PgFooDao>::new(` and `<PgFooDao as Trait>::new(`.
      while ($src =~ /<\s*(Pg[A-Za-z]+Dao)\b[^<>]*>\s*::\s*new\s*\(/g) {
        my $lineno = 1 + (substr($src, 0, $-[1]) =~ tr/\n//);
        print "$ARGV:$lineno: $1(...)\n" unless $seen{$lineno}++;
      }
    ' "$file"
  )"
  if [ -n "$matches" ]; then
    echo "$matches"
    violations=$((violations + 1))
  fi
done < <(find "$SRC" -name '*.rs' -type f | sort)

if [ "$violations" -gt 0 ]; then
  echo ""
  echo "::error::A concrete Pg*Dao is constructed outside the composition root."
  echo "Take an Arc<dyn *Dao> port from Ports/AppState instead of building one"
  echo "from a pool. See crates/backend/src/compose.rs and .plans/backend-ports-and-crates.md Step 3."
  exit 1
fi

echo "OK: no Pg*Dao construction outside the composition root."

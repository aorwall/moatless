#!/usr/bin/env bash
#
# Step 6 invariant — a table is private to the crate that owns it.
#
# A store is private to its crate; what a peer may name is a narrow port, owned
# by the domain that implements it. A production SQL statement naming a table
# another crate owns bypasses that port even where the port exists, so this
# check fails on it, naming the file and line.
#
# The check is a ratchet, in the shape `scripts/check-appstate-composition-root.sh`
# already uses: statements that predate it are listed in
# `scripts/table-ownership-baseline.txt`, and that file may only shrink. A new
# foreign statement fails immediately; removing one and not updating the
# baseline also fails, so the list cannot rot. An entry names a file and a
# table, never a line: an unrelated edit above a statement must not make the
# baseline stale.
#
# Exclusions, each deliberate and stated (not a pattern narrowed until green):
#
#   - crates/backend            the composition root. It assembles every domain
#                               and its `t3code` projection reads across them,
#                               so it names tables it does not own by design.
#   - crates/t3code             a declared read-only projection over the Task
#                               and Workspace tables. The rule it is excluded
#                               under — every statement is a read, and a read
#                               another band already publishes goes through the
#                               seam — is stated in `crates/t3code/AGENTS.md`.
#   - crates/cli, crates/sandbox
#                               neither opens a pool against this schema.
#   - #[cfg(test)] / #[cfg(all(test, ...))] modules, and module files declared
#                               only under such a gate: a test seeding a table
#                               directly is what an integration test is. The
#                               invariant protects production statements.
#   - tests/ directories        the same reason; not scanned at all.
#
# Ownership is stated below, not inferred. Every live table the migrations
# create must have exactly one owner in the map: an unmapped table fails as
# `unowned`, and a mapped table the migrations no longer create fails as
# `stale`.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BASELINE="scripts/table-ownership-baseline.txt"

# ── The map: table → owning crate ────────────────────────────────────────────
read -r -d '' OWNERS <<'MAP' || true
adapter_app_audit_events      identity
adapter_app_migrations        identity
adapter_app_secrets           identity
adapter_apps                  identity
adapter_connections           adapter-model
adapter_delivery              loops
adapter_event_receipts        loops
adapter_task_creation_claims  loops
api_keys                      identity
auth_audit_events             identity
auth_rate_limits              identity
auth_sessions                 identity
auth_transactions             identity
device_pairing_codes          identity
event_ingress_outbox          tasks
feedback                      tasks
file_metadata                 core
github_app_audit_events       identity
github_app_migrations         identity
github_apps                   identity
integration_pairing_requests  adapter-telegram
loop_configs                  loops
loops                         loops
oidc_identities               identity
plugin_activations            workspaces
plugin_skills                 workspaces
plugins                       workspaces
provider_tokens               identity
repositories                  workspaces
sandboxes                     sandbox-control
schedules                     loops
secret_audit_events           identity
secrets                       identity
subscriptions                 loops
tags                          tasks
task_activities               tasks
task_bindings                 loops
task_conversation_changes     backend-core
task_conversation_position    backend-core
task_events                   tasks
task_follows                  tasks
task_forks                    tasks
task_listing_changes          backend-core
task_listing_position         backend-core
task_message_content_blocks   tasks
task_message_contexts         tasks
task_message_usage            tasks
task_messages                 tasks
task_outbound_events          tasks
task_read_receipts            tasks
task_relations                tasks
task_tags                     tasks
task_turns                    tasks
tasks                         tasks
team_members                  identity
teams                         identity
user_credentials              identity
user_provider_ids             identity
users                         identity
workspace_credentials         workspaces
workspace_repos               workspaces
workspaces                    workspaces
MAP

EXCLUDED_CRATES="backend t3code cli sandbox"

# ── Coverage: every live table is in the map ─────────────────────────────────
# The map above is the whole of what this check enforces, and an unlisted table
# is not checked at all — so a table added without an owner is a silent hole,
# not a failure. This closes it by replaying the migrations and requiring the
# two sets to match exactly, in both directions: a live table missing an owner
# fails, and an owner for a table that no longer exists fails too, so the map
# cannot rot as the schema moves.
#
# `crates/backend/migrations` is a symlink to `crates/core/migrations`, so
# replaying the one directory covers the schema.
COVERAGE="$(python3 - "$ROOT" <<'PY'
import os, re, sys, glob

root = sys.argv[1]
script = open(os.path.join(root, "scripts/check-table-ownership.sh")).read()
body = script.split("<<'MAP'", 1)[1].split("\nMAP\n", 1)[0]
mapped = {
    m.group(1)
    for m in (re.match(r"^([a-z_][a-z0-9_-]*)\s{2,}([a-z0-9_-]+)$", l.strip()) for l in body.split("\n"))
    if m
}

CREATE = re.compile(r'create\s+table\s+(?:if\s+not\s+exists\s+)?("?[\w.]+"?)', re.I)
DROP = re.compile(r'drop\s+table\s+(?:if\s+exists\s+)?("?[\w.]+"?)', re.I)
RENAME = re.compile(r'alter\s+table\s+(?:if\s+exists\s+)?("?[\w.]+"?)\s+rename\s+to\s+("?[\w.]+"?)', re.I)


def name(raw):
    return raw.strip().strip('"').split(".")[-1].strip('"').lower()


def uncomment(sql):
    """Strip block and line comments — a table name inside prose is not a table."""
    return re.sub(r"--[^\n]*", "", re.sub(r"/\*.*?\*/", "", sql, flags=re.S))


live = set()
for path in sorted(glob.glob(os.path.join(root, "crates/core/migrations/*.sql"))):
    sql = uncomment(open(path, errors="replace").read())
    for m in CREATE.finditer(sql):
        live.add(name(m.group(1)))
    for m in RENAME.finditer(sql):
        live.discard(name(m.group(1)))
        live.add(name(m.group(2)))
    for m in DROP.finditer(sql):
        live.discard(name(m.group(1)))

for t in sorted(live - mapped):
    print(f"unowned\t{t}")
for t in sorted(mapped - live):
    print(f"stale\t{t}")
PY
)"

if [ -n "$COVERAGE" ]; then
  echo "FAIL: the table→crate map does not match the schema." >&2
  echo >&2
  printf '%s\n' "$COVERAGE" | while IFS=$'\t' read -r kind table; do
    case "$kind" in
      unowned) echo "  $table is created by a migration and has no owner in the map." >&2 ;;
      stale)   echo "  $table has an owner in the map but no longer exists." >&2 ;;
    esac
  done
  echo >&2
  echo "A table with no owner is not checked by this script at all, which is why" >&2
  echo "the map must cover the schema. Add the row to the map above, or drop the" >&2
  echo "entry if the table is gone." >&2
  exit 1
fi

# ── Pairs a written rule already settles ─────────────────────────────────────
# A `<crate> <table>` pair listed here is not a bypass but the documented shape
# of a relationship between two bands. Each names the document that states it.
read -r -d '' ALLOWED_PAIRS <<'PAIRS' || true
# Task controls Sandbox. `moatless-tasks` writes the `sandboxes` row when a task
# is created; the control plane reads a pod's shape and its access control off
# that row alone. Stated in crates/sandbox-control/AGENTS.md.
tasks                           sandboxes
# The other half of the same rule: this band reads the Task row it states intent
# on (`sandbox_id`, `status`). Stated in crates/sandbox-control/AGENTS.md.
sandbox-control                 tasks
PAIRS

# ── Scan ─────────────────────────────────────────────────────────────────────
# Written in Python: the statements are multi-line string literals, and
# recognising one needs a real tokenizer rather than a line-oriented grep.
FOUND="$(python3 - "$ROOT" "$EXCLUDED_CRATES" <<'PY'
import os, re, sys

root, excluded = sys.argv[1], set(sys.argv[2].split())

# Both tables below are read out of this file itself, so the map a reader sees
# is the map the check runs.
script = open(os.path.join(root, "scripts/check-table-ownership.sh")).read()


def block(name):
    body = script.split(f"<<'{name}'", 1)[1].split(f"\n{name}\n", 1)[0]
    out = []
    for line in body.split("\n"):
        m = re.match(r"^([a-z_][a-z0-9_-]*)\s{2,}([a-z0-9_-]+)$", line.strip("\n"))
        if m:
            out.append((m.group(1), m.group(2)))
    return out


owners = dict(block("MAP"))
allowed = set(block("PAIRS"))

KEYWORD = re.compile(r"\b(?:FROM|JOIN|INTO|UPDATE)\s+(?:ONLY\s+)?([a-z_][a-z0-9_]*)", re.I)
# Rust string literals: raw (r"…", r#"…"#, r##"…"##) and ordinary ("…").
LITERAL = re.compile(r'r(#*)"(.*?)"\1|"((?:[^"\\\n]|\\.|\\\n)*)"', re.S)


def test_only_files(src):
    """Module files reachable only under a cfg(test) gate."""
    out = set()
    for dp, _, fs in os.walk(src):
        for f in fs:
            if not f.endswith(".rs"):
                continue
            p, prev = os.path.join(dp, f), ""
            for line in open(p, errors="replace"):
                m = re.match(r"\s*(pub(\([a-z_:]+\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;", line)
                if m and re.search(r"#\[cfg\((all\()?test", prev):
                    for cand in (os.path.join(dp, m.group(3) + ".rs"),
                                 os.path.join(dp, m.group(3), "mod.rs")):
                        if os.path.exists(cand):
                            out.add(cand)
                prev = line
    return out


def blank_test_modules(text):
    """Blank the body of every inline #[cfg(test…)] module, keeping line numbers."""
    lines = text.split("\n")
    out = list(lines)
    i = 0
    while i < len(lines):
        if re.search(r"#\[cfg\((all\()?test", lines[i]):
            j = i + 1
            while j < len(lines) and not re.search(r"\b(mod|fn|impl|struct|use)\b", lines[j]):
                j += 1
            if j < len(lines) and re.search(r"\bmod\b.*\{", lines[j]):
                depth = lines[j].count("{") - lines[j].count("}")
                out[j] = ""
                k = j + 1
                while k < len(lines) and depth > 0:
                    depth += lines[k].count("{") - lines[k].count("}")
                    out[k] = ""
                    k += 1
                i = k
                continue
        i += 1
    return "\n".join(out)


crates = []
for c in sorted(os.listdir(os.path.join(root, "crates"))):
    s = os.path.join(root, "crates", c, "src")
    if os.path.isdir(s):
        crates.append((c, s))

findings = {}
for crate, src in crates:
    if crate in excluded:
        continue
    skip = test_only_files(src)
    for dp, _, fs in os.walk(src):
        for f in sorted(fs):
            if not f.endswith(".rs"):
                continue
            p = os.path.join(dp, f)
            if p in skip:
                continue
            text = blank_test_modules(open(p, errors="replace").read())
            for m in LITERAL.finditer(text):
                body = m.group(2) if m.group(2) is not None else (m.group(3) or "")
                for table in KEYWORD.findall(body):
                    table = table.lower()
                    owner = owners.get(table)
                    if owner and owner != crate and (crate, table) not in allowed:
                        line = text.count("\n", 0, m.start()) + 1
                        rel = os.path.relpath(p, root)
                        # Keyed on file and table, never the line: an unrelated
                        # edit above a statement must not make the baseline stale.
                        findings.setdefault(f"{rel}\t{table}\t{owner}", []).append(line)

print("\n".join(f"{k}\t{','.join(str(n) for n in sorted(v))}" for k, v in sorted(findings.items())))
PY
)"

# ── Compare against the baseline ─────────────────────────────────────────────
if [ ! -f "$BASELINE" ]; then
  echo "FAIL: $BASELINE is missing." >&2
  exit 1
fi

BASELINE_ENTRIES="$(grep -vE '^\s*(#|$)' "$BASELINE" || true)"

# The line numbers ride along in the scan output for the failure message only;
# both sides are compared on file and table.
FOUND_KEYS="$(printf '%s\n' "$FOUND" | grep -v '^$' | cut -f1-3 | sort || true)"

NEW="$(comm -23 <(printf '%s\n' "$FOUND_KEYS") \
                <(printf '%s\n' "$BASELINE_ENTRIES" | sort) || true)"
GONE="$(comm -13 <(printf '%s\n' "$FOUND_KEYS") \
                 <(printf '%s\n' "$BASELINE_ENTRIES" | sort) || true)"

status=0

if [ -n "$NEW" ]; then
  echo "FAIL: production SQL against a table another crate owns:" >&2
  printf '%s\n' "$NEW" | while IFS=$'\t' read -r file table owner; do
    lines="$(printf '%s\n' "$FOUND" | awk -F'\t' -v f="$file" -v t="$table" '$1==f && $2==t {print $4}')"
    echo "  $file:$lines  reads '$table', owned by crates/$owner" >&2
  done
  echo >&2
  echo "Route it through the port that domain publishes. If there is none, declare one" >&2
  echo "in the owning crate (or in moatless-backend-core when the caller sits below it)." >&2
  status=1
fi

if [ -n "$GONE" ]; then
  echo "FAIL: $BASELINE lists statements that no longer exist:" >&2
  printf '%s\n' "$GONE" | while IFS=$'\t' read -r file table owner; do
    echo "  $file  ($table)" >&2
  done
  echo >&2
  echo "Delete those lines. The baseline only shrinks, and a stale entry hides a regression." >&2
  status=1
fi

if [ "$status" -eq 0 ]; then
  count="$(printf '%s\n' "$BASELINE_ENTRIES" | grep -c . || true)"
  echo "OK: no new foreign-table statement; $count still listed in $BASELINE."
fi

exit "$status"

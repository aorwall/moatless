#!/usr/bin/env bash
# Per-workspace sandbox memory report: measure -> attribute -> verdict.
#
# Read-only. Joins Prometheus RSS metrics to workspaces via task ID, because pod
# names carry moatless.io/task-id and workspace *names* are not unique.
set -euo pipefail

WINDOW=24h
NAMESPACE=moatless
DEFAULT_LIMIT=""           # resolved from the cluster unless --default-limit is given
DEFAULT_REQUEST=""         # likewise, unless --default-request is given
FALLBACK_LIMIT=16Gi        # backend config.rs + Helm chart default, if the cluster can't be read
FALLBACK_REQUEST=2Gi       # same source: env_str("K8S_RESOURCE_REQUEST_MEMORY", "2Gi")
GRAFANA=https://g-0000000000.grafana-workspace.eu-central-1.amazonaws.com

usage() {
  cat <<EOF
Usage: workspace-report.sh [--window 24h] [--namespace moatless]
                           [--default-limit 16Gi] [--default-request 2Gi]
                           [--grafana URL]

Reports median and peak container_memory_rss per workspace against configured
requests and limits, with a sizing verdict for each.

Workspaces without their own request or limit inherit the deployment's. Those
values differ per environment, so they are read from the backend's
K8S_RESOURCE_REQUEST_MEMORY and K8S_RESOURCE_LIMIT_MEMORY when kubectl is
available; --default-request/--default-limit override, and $FALLBACK_REQUEST /
$FALLBACK_LIMIT are the last resort. Guessing the limit low inflates every "% of
limit" and invents RAISE verdicts; treating an inherited request as zero
suppresses every request verdict, which is what most rows inherit.

Requires: GRAFANA_TOKEN (see .envrc), the 'moat' CLI, jq. kubectl optional.
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --window)        WINDOW=$2; shift 2 ;;
    --namespace)     NAMESPACE=$2; shift 2 ;;
    --default-limit) DEFAULT_LIMIT=$2; shift 2 ;;
    --default-request) DEFAULT_REQUEST=$2; shift 2 ;;
    --grafana)       GRAFANA=$2; shift 2 ;;
    -h|--help)       usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[ -n "${GRAFANA_TOKEN:-}" ] || {
  echo "GRAFANA_TOKEN is not set — export it in .envrc" >&2; exit 1; }
command -v jq >/dev/null || { echo "jq not found" >&2; exit 1; }

# The CLI may be on PATH or built locally in the moatless checkout.
# Pick a moat binary that actually serves these subcommands. An older installed
# CLI on PATH may predate `workspaces`, so probe rather than trust the first hit.
MOAT=""
for c in "$(command -v moat || true)" ./target/debug/moat ./target/release/moat; do
  [ -n "$c" ] && [ -x "$c" ] || continue
  if "$c" workspaces list --json >/dev/null 2>&1; then MOAT=$c; break; fi
done
if [ -z "$MOAT" ]; then
  echo "No moat CLI supporting 'workspaces list --json' was found." >&2
  echo "Tried PATH and ./target/{debug,release}/moat. Either the CLI predates" >&2
  echo "the subcommand or you are not logged in ('moat login')." >&2
  echo "From a moatless checkout: cargo build -p moatless-cli --bin moat" >&2
  exit 1
fi

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

# The inherited request and limit are whatever the backend was deployed with, so
# prefer the live values over any constant compiled in here.
backend_env() {
  kubectl -n "$NAMESPACE" get deploy moatless-vibe-backend \
    -o jsonpath="{.spec.template.spec.containers[0].env[?(@.name==\"$1\")].value}" \
    2>/dev/null || true
}

if [ -n "$DEFAULT_LIMIT" ]; then
  LIMIT_SOURCE="--default-limit"
else
  DEFAULT_LIMIT=$(backend_env K8S_RESOURCE_LIMIT_MEMORY)
  if [ -n "$DEFAULT_LIMIT" ]; then
    LIMIT_SOURCE="cluster K8S_RESOURCE_LIMIT_MEMORY"
  else
    DEFAULT_LIMIT=$FALLBACK_LIMIT
    LIMIT_SOURCE="built-in fallback — verify against the target environment"
  fi
fi

if [ -n "$DEFAULT_REQUEST" ]; then
  REQUEST_SOURCE="--default-request"
else
  DEFAULT_REQUEST=$(backend_env K8S_RESOURCE_REQUEST_MEMORY)
  if [ -n "$DEFAULT_REQUEST" ]; then
    REQUEST_SOURCE="cluster K8S_RESOURCE_REQUEST_MEMORY"
  else
    DEFAULT_REQUEST=$FALLBACK_REQUEST
    REQUEST_SOURCE="built-in fallback — verify against the target environment"
  fi
fi

promq() {
  curl -s -G "$GRAFANA/api/datasources/proxy/uid/prometheus/api/v1/query" \
    -H "Authorization: Bearer $GRAFANA_TOKEN" --data-urlencode "query=$1"
}

# pod -> taskid-prefix, MiB. The pod name embeds moatless.io/task-id, not
# workspace id; the prefix rides along as a column so the metric join can key on
# the pod, which is the only thing unique per series.
extract() {
  jq -r '.data.result[]
         | select(.metric.pod != null)
         | [.metric.pod,
            (.metric.pod | capture("moatless-ws-(?<t>[0-9a-f]{8})").t),
            ((.value[1] | tonumber) / 1048576 | round)]
         | @tsv' | sort -k1,1
}

SEL="{namespace=\"$NAMESPACE\",container=\"sandbox\"}"
promq "max_over_time(container_memory_rss${SEL}[$WINDOW])"           | extract > "$TMP/peak.tsv"
promq "quantile_over_time(0.5, container_memory_rss${SEL}[$WINDOW])" | extract > "$TMP/med.tsv"

if [ ! -s "$TMP/peak.tsv" ]; then
  echo "No container_memory_rss series for namespace=$NAMESPACE over $WINDOW." >&2
  echo "Check the Prometheus keep-list retains container_memory_rss." >&2
  exit 1
fi

# taskid-prefix -> workspaceId. Paginate; stop at the first empty page.
: > "$TMP/tasks.tsv"
for off in 0 200 400 600 800 1000 1200 1400 1600; do
  # Staged rather than piped: under pipefail a failing page would abort the run.
  "$MOAT" tasks list --json --limit 200 --offset "$off" >"$TMP/page.json" 2>/dev/null || break
  jq -r '.items[]? | [(.taskId|.[0:8]), .workspaceId] | @tsv' <"$TMP/page.json" >"$TMP/page.tsv" || break
  [ -s "$TMP/page.tsv" ] || break
  cat "$TMP/page.tsv" >> "$TMP/tasks.tsv"
done
sort -u -k1,1 "$TMP/tasks.tsv" -o "$TMP/tasks.tsv"

if [ ! -s "$TMP/tasks.tsv" ]; then
  echo "No tasks returned by the moat CLI — cannot map pods to workspaces." >&2
  exit 1
fi

# workspaceId -> name, req_mem, lim_mem, req_cpu. Keyed by ID: names collide.
"$MOAT" workspaces list --json 2>/dev/null \
  | jq -r '.[] | [.id, .name,
                  (.resources.requests.memory // "-"),
                  (.resources.limits.memory   // "-"),
                  (.resources.requests.cpu    // "-")] | @tsv' \
  | sort -k1,1 > "$TMP/ws.tsv"

# peak + median per pod, then aggregate to the workspace. Join the two metrics on
# the pod and never on the task prefix: a task outlives many pods, both sides
# carry every one of them, so a prefix join is a cartesian product whose row
# count squares per metric added (n=43848 for 18 tasks, once).
TAB=$(printf '\t')
join -t"$TAB" -o '0,1.2,1.3,2.3' "$TMP/peak.tsv" "$TMP/med.tsv" > "$TMP/perpod.tsv"
sort -t"$TAB" -k2,2 "$TMP/perpod.tsv" -o "$TMP/perpod.tsv"

# N counts distinct tasks, not pods: the report reads it as sample size, and one
# task rescheduled across ten nodes is still one sample.
join -t"$TAB" -1 1 -2 2 -o '1.2,2.2,2.3,2.4' "$TMP/tasks.tsv" "$TMP/perpod.tsv" \
  | awk -F'\t' '{ id=$1; if ($3+0 > pk[id]) pk[id]=$3;
                          if ($4+0 > md[id]) md[id]=$4
                  if (!((id SUBSEP $2) in seen)) { seen[id SUBSEP $2]=1; n[id]++ } }
                 END { for (i in pk) printf "%s\t%d\t%d\t%d\n", i, pk[i], md[i], n[i] }' \
  | sort -k1,1 > "$TMP/byws.tsv"

printf '  %-32s %8s %8s %9s %8s %7s  %s\n' \
  WORKSPACE MEDIAN PEAK REQUEST LIMIT "OF_LIM" VERDICT

join -t"$(printf '\t')" -a1 -e '?' -o '0,1.2,1.3,1.4,2.2,2.3,2.4' \
     "$TMP/byws.tsv" "$TMP/ws.tsv" \
  | sort -t"$(printf '\t')" -k2,2 -rn \
  | awk -F'\t' -v deflim="$DEFAULT_LIMIT" -v defreq="$DEFAULT_REQUEST" '
      function unset(s) { return (s == "-" || s == "?" || s == "") }
      function mib(s,   n) {
        n = s + 0
        if (s ~ /Gi$/) return n * 1024
        if (s ~ /Mi$/) return n
        if (s ~ /Ki$/) return n / 1024
        return n / 1048576            # bare bytes
      }
      {
        peak = $2; med = $3; tasks = $4
        name = ($5 == "?" ? "(deleted workspace)" : $5)
        req  = $6; lim = $7
        limshown = unset(lim) ? deflim "*" : lim
        reqshown = unset(req) ? defreq "*" : req
        # An unset request is inherited, not absent. Reading it as 0 MiB is what
        # suppressed every request verdict on the rows that inherit — most of them.
        limmi = unset(lim) ? mib(deflim) : mib(lim)
        reqmi = unset(req) ? mib(defreq) : mib(req)
        pct   = limmi > 0 ? peak / limmi * 100 : 0

        # 10% of slack: a row sitting a megabyte over its request would
        # otherwise flap in and out of the verdict between runs.
        if (pct > 85)                                     v = "RAISE LIMIT"
        else if (reqmi > 0 && med > 1.1*reqmi)            v = "RAISE REQUEST"
        else if (reqmi > 512 && med > 0 && reqmi > 4*med) v = "LOWER REQUEST"
        else                                              v = "ok"
        if (tasks < 3) v = v "  (n=" tasks ", lower bound)"

        printf "  %-32s %6dMi %6dMi %9s %8s %6.0f%%  %s\n",
               substr(name, 1, 32), med, peak, reqshown, limshown, pct, v
      }'

echo "  * inherited: request $DEFAULT_REQUEST [$REQUEST_SOURCE], limit $DEFAULT_LIMIT [$LIMIT_SOURCE]"
echo "  window=$WINDOW namespace=$NAMESPACE — requests track MEDIAN, limits track PEAK"

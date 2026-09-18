#!/usr/bin/env node
// Collect one UTC day of moatless health data into JSON files, and print the
// digest the report's stat tiles are built from.
//
//   node collect.mjs 2026-09-16 [--out ./data] [--only api|stats|loki]
//
// Needs GRAFANA_TOKEN in the environment and an authenticated `moat`. Every
// number it prints is also written to <out>/digest.json; nothing here decides
// what is worth reporting.

import { execFile } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { promisify } from "node:util";

const exec = promisify(execFile);
const GRAFANA = "https://g-0000000000.grafana-workspace.eu-central-1.amazonaws.com";
const NS = "moatless";

const argv = process.argv.slice(2);
const DATE = argv.find((a) => /^\d{4}-\d{2}-\d{2}$/.test(a));
const OUT = argv[argv.indexOf("--out") + 1] ?? "./data";
const ONLY = argv.includes("--only") ? argv[argv.indexOf("--only") + 1] : null;
if (!DATE) {
  console.error("usage: collect.mjs <YYYY-MM-DD> [--out DIR] [--only api|stats|loki]");
  process.exit(2);
}
if (!process.env.GRAFANA_TOKEN) {
  console.error("GRAFANA_TOKEN is not set — export it before running (it is in .envrc).");
  process.exit(2);
}

const DAY0 = Date.parse(`${DATE}T00:00:00Z`) / 1000;
const DAY1 = DAY0 + 86400;
const run = (what) => !ONLY || ONLY === what;
const write = async (name, data) => {
  await writeFile(`${OUT}/${name}.json`, JSON.stringify(data, null, 1));
  return data;
};

// ------------------------------------------------------------- moatless REST

async function api(path) {
  try {
    const { stdout } = await exec("moat", ["api", "GET", path, "--json"], {
      timeout: 180_000,
      maxBuffer: 64 * 1024 * 1024,
    });
    return JSON.parse(stdout);
  } catch (e) {
    return { status: -1, body: null, err: String(e).slice(0, 300) };
  }
}

/** Rows one collection will pull at most. Above this the API paginates for
 * minutes, and a day inside the last month never needs it. */
const CAP = 5000;

/** Walk an offset-paginated collection. `cap` bounds a runaway page loop. */
async function paginate(base, key = "items", page = 200, cap = CAP) {
  const out = [];
  for (let offset = 0; offset < cap; offset += page) {
    const sep = base.includes("?") ? "&" : "?";
    const r = await api(`${base}${sep}limit=${page}&offset=${offset}`);
    if (r.status !== 200) {
      console.error(`  ! ${base} offset=${offset} -> ${r.status} ${r.err ?? ""}`);
      break;
    }
    const body = r.body ?? {};
    const items = Array.isArray(body) ? body : (body[key] ?? []);
    out.push(...items);
    if (items.length < page) break;
  }
  return out;
}

/**
 * Report the cap only when it cut into the day being collected. Hitting it is
 * normal and says nothing on its own; the day is truncated only if the oldest
 * row pulled is itself newer than the window.
 */
function checkCoverage(name, rows) {
  if (rows.length < CAP) return;
  const oldest = Math.min(...rows.map((r) => Date.parse(r.createdAt)));
  if (oldest / 1000 > DAY0)
    console.error(
      `  ! ${name}: hit the ${CAP}-row cap and the oldest row is ${new Date(oldest).toISOString().slice(0, 10)} — ${DATE} is truncated, raise CAP`,
    );
}

const unwrap = (r) => {
  const b = r.body ?? [];
  return Array.isArray(b) ? b : (b.items ?? []);
};

// ---------------------------------------------------------------------- Loki

async function loki(query, { step = 3600, limit = null, start = DAY0 + 3600, end = DAY1 } = {}) {
  const p = new URLSearchParams({ query, start: String(start), end: String(end) });
  if (limit) {
    p.set("limit", String(limit));
    p.set("direction", "backward");
  } else {
    p.set("step", String(step));
  }
  const res = await fetch(`${GRAFANA}/api/datasources/proxy/uid/loki/loki/api/v1/query_range?${p}`, {
    headers: { Authorization: `Bearer ${process.env.GRAFANA_TOKEN}` },
  });
  const text = await res.text();
  let d;
  try {
    d = JSON.parse(text);
  } catch {
    console.error(`  ! loki: ${text.slice(0, 200)}`);
    return [];
  }
  if (d.status !== "success") {
    console.error(`  ! loki: ${JSON.stringify(d).slice(0, 200)}`);
    return [];
  }
  return d.data.result ?? [];
}

/**
 * Fold a `count_over_time(...[1h])` matrix into 24 hourly buckets keyed by one
 * label. A sample at t covers (t-1h, t], so it belongs to the hour before it —
 * label it any other way and the whole series reads one hour late.
 */
function hourly(result, label) {
  const out = {};
  for (const r of result) {
    const key = label ? (r.metric[label] ?? "_") : "_";
    const row = (out[key] ??= new Array(24).fill(0));
    for (const [ts, v] of r.values) row[new Date((ts - 3600) * 1000).getUTCHours()] += Number(v);
  }
  return out;
}

const sum = (xs) => xs.reduce((a, b) => a + b, 0);
const totals = (obj) => Object.fromEntries(Object.entries(obj).map(([k, v]) => [k, sum(v)]));

// ------------------------------------------------------------------- the day

const inWindow = (s) => {
  if (!s) return false;
  const t = Date.parse(s) / 1000;
  return t >= DAY0 && t < DAY1;
};

async function main() {
  await mkdir(OUT, { recursive: true });
  const digest = { date: DATE };
  // --only re-runs one group into a digest the other groups already wrote, so
  // start from what is there. Without this a re-run leaves the rest as holes.
  if (ONLY)
    try {
      const prev = JSON.parse(await readFile(`${OUT}/digest.json`, "utf8"));
      // `prev` carries its own `date`, so merging another day's file would stamp
      // this digest with that date and hand the builder a mixed-date page.
      if (prev.date === DATE) Object.assign(digest, prev);
      else console.error(`  ! ${OUT}/digest.json is ${prev.date}, not ${DATE} — writing ${ONLY} alone`);
    } catch {
      console.error(`  ! no digest to merge into at ${OUT}/digest.json — writing ${ONLY} alone`);
    }

  let tasks = [];
  let touched = [];
  if (run("api") || run("stats")) {
    tasks = await write("tasks", await paginate("/api/v1/tasks"));
    checkCoverage("tasks", tasks);
    const created = tasks.filter((t) => inWindow(t.createdAt));
    // A task still being worked after midnight keeps moving `lastMessageAt`
    // forward, so "last message inside the window" drops it as soon as it
    // speaks again the next day — the same date collected twice then gives two
    // different answers. Take every task whose last message is at or after the
    // window opens, and let the per-message timestamps in the stats group say
    // which of them actually ran in it.
    const carried = tasks.filter(
      (t) =>
        !inWindow(t.createdAt) &&
        Date.parse(t.createdAt) < DAY1 * 1000 &&
        (t.lastMessageAt ? Date.parse(t.lastMessageAt) : 0) >= DAY0 * 1000,
    );
    touched = [...created, ...carried];
    const createdPerHour = new Array(24).fill(0);
    for (const t of created) createdPerHour[new Date(t.createdAt).getUTCHours()]++;
    // Spread rather than replace: `touched`, `error`, `costTouchedUsd` and
    // `carriedPerHour` belong to the stats group, which is the only place the
    // candidates above are narrowed to the tasks that ran. Under `--only api`
    // that group does not run, and assigning a fresh object would drop them.
    digest.tasks = {
      ...digest.tasks,
      created: created.length,
      createdPerHour,
      closedSameDay: created.filter((t) => t.status === "closed").length,
      costCreatedUsd: +created.reduce((a, t) => a + (t.totalCostUsd ?? 0), 0).toFixed(2),
      byAgentType: tally(created, (t) => t.agentType),
      byRepo: tally(created, (t) => t.repositoryName),
    };
    console.log(
      `tasks: ${tasks.length} (${created.length} created, ${carried.length} candidates carried in)`,
    );
  }

  if (run("api")) {
    const feedback = await write("feedback", await paginate("/api/v1/feedback"));
    checkCoverage("feedback", feedback);
    const day = feedback.filter((f) => inWindow(f.createdAt));
    digest.feedback = { filed: day.length, byCategory: tally(day, (f) => f.category) };
    await write("loops", unwrap(await api("/api/v1/loops")));
    await write("workspaces", unwrap(await api("/api/v1/workspaces")));
    await write("repos", unwrap(await api("/api/v1/repositories")));
    console.log(`feedback: ${day.length} filed in window`);
  }

  if (run("stats")) {
    const rows = await pool(touched, 8, async (t) => {
      const s = (await api(`/api/v1/tasks/${t.taskId}/statistics`)).body ?? {};
      const msgs = (s.messageTokenUsage ?? []).filter((m) => inWindow(m.timestamp));
      return {
        taskId: t.taskId,
        name: t.name ?? t.title,
        repo: t.repositoryName,
        agentType: t.agentType,
        status: t.status,
        createdInWindow: inWindow(t.createdAt),
        lastMessageHour: inWindow(t.lastMessageAt) ? new Date(t.lastMessageAt).getUTCHours() : null,
        totalCostUsd: t.totalCostUsd,
        msgsInWindow: msgs.length,
        turnsInWindow: Math.max(0, ...msgs.map((m) => m.turnNumber ?? 0)),
        tokensInWindow: sum(msgs.map((m) => m.totalTokens ?? 0)),
        outputTokensInWindow: sum(msgs.map((m) => m.outputTokens ?? 0)),
        cachedInWindow: sum(msgs.map((m) => m.cachedInputTokens ?? 0)),
        hourOfMsgs: msgs.map((m) => new Date(m.timestamp).getUTCHours()),
        toolCalls: s.toolCalls ?? [],
      };
    });
    await write("stats", rows);
    // A carried task counts once it shows any activity in the window, and it
    // belongs to the hour it resumed rather than to a last message that may now
    // sit on a later day. `messageTokenUsage` records agent turns only, so a
    // task whose window activity was a person writing to it has none —
    // `lastMessageHour` catches those.
    const resumed = new Set(
      rows.filter((r) => !r.createdInWindow && (r.msgsInWindow > 0 || r.lastMessageHour !== null)),
    );
    // Every day total reads `inDay`: the candidates the API group hands over
    // include tasks that were idle on DATE and only spoke again later.
    const inDay = rows.filter((r) => r.createdInWindow || resumed.has(r));
    const carriedPerHour = new Array(24).fill(0);
    for (const r of resumed)
      carriedPerHour[r.hourOfMsgs.length ? Math.min(...r.hourOfMsgs) : r.lastMessageHour]++;
    digest.tasks.touched = inDay.length;
    digest.tasks.carriedPerHour = carriedPerHour;
    digest.tasks.error = inDay.filter((r) => r.status === "error").length;
    digest.tasks.costTouchedUsd = +sum(inDay.map((r) => r.totalCostUsd ?? 0)).toFixed(2);

    const tools = {};
    for (const r of inDay)
      for (const c of r.toolCalls) {
        const e = (tools[c.toolName] ??= { calls: 0, failed: 0 });
        e.calls += c.totalCalls ?? 0;
        e.failed += c.failedCalls ?? 0;
      }
    const msgsPerHour = new Array(24).fill(0);
    for (const r of rows) for (const h of r.hourOfMsgs) msgsPerHour[h]++;
    digest.agents = {
      messages: sum(rows.map((r) => r.msgsInWindow)),
      turns: sum(rows.map((r) => r.turnsInWindow)),
      tokens: sum(rows.map((r) => r.tokensInWindow)),
      outputTokens: sum(rows.map((r) => r.outputTokensInWindow)),
      cachedShare: pct(sum(rows.map((r) => r.cachedInWindow)), sum(rows.map((r) => r.tokensInWindow))),
      msgsPerHour,
      tools,
      toolFailureRate: pct(
        sum(Object.values(tools).map((t) => t.failed)),
        sum(Object.values(tools).map((t) => t.calls)),
      ),
    };
    console.log(`stats: ${rows.length} tasks, ${digest.agents.messages} messages`);
  }

  if (run("loki")) {
    // Warning events only: the eventhandler stream carries Normal ones too, and
    // they outnumber the warnings, so an unfiltered count reads as a disaster.
    const ev = hourly(
      await loki(
        `sum by (reason) (count_over_time({namespace="${NS}", service_name="integrations/kubernetes/eventhandler"} ` +
          "| regexp `reason=(?P<reason>\\S+) type=(?P<etype>\\S+)` | etype=\"Warning\" [1h]))",
      ),
      "reason",
    );
    await write("k8s_events", ev);
    digest.k8sWarnings = { total: sum(Object.values(ev).map(sum)), byReason: sortDesc(totals(ev)) };

    // One query answers both pod questions — how many ran at once, and how many
    // a sandbox burned through. A pod is named moatless-ws-<task-id>-<hash>, so
    // the task id is what groups pods into one sandbox.
    const podSeries = await loki(
      `count by (pod) (count_over_time({namespace="${NS}", service_name="moatless-sandbox"}[1h]))`,
    );
    const podsPerHour = new Array(24).fill(0);
    const podsByTask = {};
    for (const s of podSeries) {
      for (const [ts] of s.values) podsPerHour[new Date((ts - 3600) * 1000).getUTCHours()]++;
      const id = s.metric.pod.match(/^moatless-ws-([0-9a-f-]{36})/)?.[1] ?? s.metric.pod;
      (podsByTask[id] ??= []).push(s.metric.pod);
    }
    await write("sandbox_pods", { podsPerHour, podsByTask });
    digest.sandboxPods = {
      pods: podSeries.length,
      sandboxes: Object.keys(podsByTask).length,
      multiPod: Object.values(podsByTask).filter((p) => p.length > 1).length,
      peakConcurrent: Math.max(0, ...podsPerHour),
      podsPerHour,
    };

    const levels = {};
    for (const svc of ["moatless", "moatless-sandbox"])
      levels[svc] = hourly(
        await loki(
          `sum by (log_level) (count_over_time({namespace="${NS}", service_name="${svc}"}[1h]))`,
        ),
        "log_level",
      );
    await write("log_levels", levels);
    digest.logLines = Object.fromEntries(
      Object.entries(levels).map(([svc, byLevel]) => [svc, sortDesc(totals(byLevel))]),
    );

    const http = await loki(
      `sum by (path, status) (count_over_time({namespace="${NS}", service_name="moatless"} ` +
        '| json path="span.path", status="span.status" | status =~ "5[0-9][0-9]" [1h]))',
    );
    const by5xx = http.map((r) => ({
      path: r.metric.path,
      status: r.metric.status,
      count: sum(r.values.map(([, v]) => Number(v))),
    }));
    await write("http_5xx", by5xx);
    digest.http5xx = { total: sum(by5xx.map((r) => r.count)), rows: by5xx.length };

    const patterns = {
      rustic_xattr: "setting extended attributes failed",
      cli_unknown_frame: "CLI stdout: unknown type",
      review_git_failed: "git command for the review surface failed",
      system_time_zone: "failed to get system time zone",
      slow_statement: "slow statement",
      sandbox_slow_start: "Sandbox container slow to start",
    };
    const noise = {};
    for (const [name, needle] of Object.entries(patterns))
      noise[name] = sum(
        Object.values(
          hourly(
            await loki(
              `sum(count_over_time({namespace="${NS}"} |= ${JSON.stringify(needle)} [1h]))`,
            ),
            null,
          ),
        ).map(sum),
      );
    await write("patterns", noise);
    digest.noisePatterns = sortDesc(noise);
    console.log(
      `loki: ${digest.k8sWarnings.total} warning events, ${digest.http5xx.total} 5xx, ` +
        `${digest.sandboxPods.pods} pods across ${digest.sandboxPods.sandboxes} sandboxes`,
    );
  }

  await write("digest", digest);
  console.log(`\n--- digest (${OUT}/digest.json) ---`);
  console.log(JSON.stringify(digest, null, 1));
}

// ------------------------------------------------------------------ plumbing

function tally(rows, key) {
  const out = {};
  for (const r of rows) {
    const k = key(r) ?? "(none)";
    out[k] = (out[k] ?? 0) + 1;
  }
  return sortDesc(out);
}

const sortDesc = (obj) =>
  Object.fromEntries(Object.entries(obj).sort((a, b) => b[1] - a[1]));

const pct = (part, whole) => (whole ? +((part / whole) * 100).toFixed(1) : 0);

/** Run `fn` over `items` with at most `n` in flight — the API rate-limits above ~8. */
async function pool(items, n, fn) {
  const out = new Array(items.length);
  let i = 0;
  await Promise.all(
    new Array(Math.min(n, items.length)).fill(0).map(async () => {
      while (i < items.length) {
        const k = i++;
        out[k] = await fn(items[k]);
      }
    }),
  );
  return out;
}

await main();

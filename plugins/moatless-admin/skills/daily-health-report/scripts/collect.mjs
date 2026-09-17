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
import { mkdir, writeFile } from "node:fs/promises";
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

/** Walk an offset-paginated collection. `cap` bounds a runaway page loop. */
async function paginate(base, key = "items", page = 200, cap = 5000) {
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
  if (out.length >= cap)
    console.error(`  ! ${base} stopped at the ${cap}-row cap — a day older than the newest ${cap} rows is truncated`);
  return out;
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

  let tasks = [];
  if (run("api") || run("stats")) {
    tasks = await write("tasks", await paginate("/api/v1/tasks"));
    const created = tasks.filter((t) => inWindow(t.createdAt));
    const touched = tasks.filter((t) => inWindow(t.createdAt) || inWindow(t.lastMessageAt));
    // Two series for one chart: new tasks, and tasks created earlier whose last
    // message landed in the window. Together they are `touched`.
    const createdPerHour = new Array(24).fill(0);
    const carriedPerHour = new Array(24).fill(0);
    for (const t of touched) {
      if (inWindow(t.createdAt)) createdPerHour[new Date(t.createdAt).getUTCHours()]++;
      else carriedPerHour[new Date(t.lastMessageAt).getUTCHours()]++;
    }
    digest.tasks = {
      created: created.length,
      touched: touched.length,
      createdPerHour,
      carriedPerHour,
      closedSameDay: created.filter((t) => t.status === "closed").length,
      error: touched.filter((t) => t.status === "error").length,
      costCreatedUsd: +created.reduce((a, t) => a + (t.totalCostUsd ?? 0), 0).toFixed(2),
      costTouchedUsd: +touched.reduce((a, t) => a + (t.totalCostUsd ?? 0), 0).toFixed(2),
      byAgentType: tally(created, (t) => t.agentType),
      byRepo: tally(created, (t) => t.repositoryName),
    };
    console.log(`tasks: ${tasks.length} (${created.length} created, ${touched.length} touched)`);
  }

  if (run("api")) {
    const feedback = await write("feedback", await paginate("/api/v1/feedback"));
    const day = feedback.filter((f) => inWindow(f.createdAt));
    digest.feedback = { filed: day.length, byCategory: tally(day, (f) => f.category) };
    await write("loops", unwrap(await api("/api/v1/loops")));
    await write("workspaces", unwrap(await api("/api/v1/workspaces")));
    await write("repos", unwrap(await api("/api/v1/repositories")));
    console.log(`feedback: ${day.length} filed in window`);
  }

  if (run("stats")) {
    const touched = tasks.filter((t) => inWindow(t.createdAt) || inWindow(t.lastMessageAt));
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
    const tools = {};
    for (const r of rows)
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

    const pods = hourly(
      await loki(
        `count(count by (pod) (count_over_time({namespace="${NS}", service_name="moatless-sandbox"}[1h])))`,
      ),
      null,
    );
    await write("sandbox_pods", pods._ ?? []);
    digest.sandboxPodsPeak = Math.max(...(pods._ ?? [0]));

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
        `peak ${digest.sandboxPodsPeak} concurrent sandbox pods`,
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

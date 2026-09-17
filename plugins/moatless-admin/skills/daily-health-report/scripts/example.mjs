#!/usr/bin/env node
// Render the reference page: the stat tiles, the "Tasks, metrics and cost"
// cards, and the findings and feedback sections, as the 2026-09-16 report has
// them.
//
//   node example.mjs [outfile]      # default: scratchpad/layout-smoke.html
//
// The numbers are that day's real ones, baked in so this runs with no data
// directory, no GRAFANA_TOKEN and no network. Read it as the worked example of
// what a `build.mjs` looks like; a real one reads the same shapes out of
// digest.json.

import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { card, columns, hbars, num, page, section, swatch, table, tiles } from "./layout.mjs";

const OUT = process.argv[2] ?? "/opt/moatless/workspace/scratchpad/layout-smoke.html";
const HERE = dirname(fileURLToPath(import.meta.url));

// The two prose sections are plain HTML beside the generated cards — the pattern
// the skill prescribes for a section no component draws. Read from disk rather
// than inlined so a template literal never has to escape their backticks.
const [findingsHtml, feedbackHtml] = await Promise.all([
  readFile(join(HERE, "example-findings.html"), "utf8"),
  readFile(join(HERE, "example-feedback.html"), "utf8"),
]);

const HOURS = [...Array(24).keys()].map((h) => String(h).padStart(2, "0"));
const CLOCK = HOURS.map((h) => `${h}:00`);

// Tasks by the hour they were created, and tasks created earlier whose last
// message landed in the window — the two together are the 117 touched.
const CREATED = [0, 0, 0, 1, 4, 3, 10, 2, 4, 6, 2, 11, 6, 5, 13, 7, 5, 3, 3, 2, 0, 1, 2, 0];
const CARRIED = [0, 0, 0, 2, 0, 3, 1, 0, 1, 0, 2, 2, 6, 3, 1, 0, 0, 0, 0, 0, 1, 4, 1, 0];

// Warning events only, bucketed by the hour each sample covers. Hour 23 is zero
// because the sample at the next midnight closes the day rather than opening it.
const WARNINGS = {
  FailedScheduling: [0, 0, 10, 5, 62, 81, 66, 76, 203, 105, 130, 23, 9, 0, 59, 204, 0, 5, 7, 0, 0, 0, 0, 0],
  Unhealthy: [0, 0, 3, 11, 33, 97, 53, 41, 103, 49, 46, 25, 42, 29, 48, 35, 36, 25, 20, 28, 9, 13, 11, 0],
  NodeNotReady: [0, 0, 0, 0, 13, 37, 9, 37, 43, 20, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  BackOff: [0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 15, 3, 7, 18, 0, 9, 23, 0, 0],
};

const SHAPE = [
  ["FailedScheduling", "204 @ 15:00", "Two spikes, 08:00 and 15:00; silent after 18:00"],
  ["Unhealthy", "103 @ 08:00", "Present every hour 02:00–22:00; startup probe 503s"],
  ["NodeNotReady", "43 @ 08:00", "Confined to 04:00–10:00, then stops entirely"],
  ["BackOff", "23 @ 21:00", "Afternoon only; mostly one crash-looping task"],
];

const LOOPS = [
  ["Full-Repo Code Review", "adapter event", 30],
  ["frontend-review", "adapter event", 14],
  [`java-review <span class="muted">(4 instances)</span>`, "adapter event", 9],
  ["spec-test-review", "adapter event", 7],
  ["Linear triage (SP)", "adapter event", 5],
  ["Linear triage (OP)", "adapter event", 1],
  ["Upstream Merge", "schedule", 1],
  ["Agent CLI Upgrade", "schedule", 1],
  ["daily-health-check", "schedule", 1],
  ["slack-default", "adapter event", 1],
];

// Cost carried by tasks created in the window, so a long-running task created
// yesterday spends here without inflating today's figure.
const COST = [
  ["moatless", 40, 226.0],
  ["soap-specs", 14, 137.52],
  ["soap-agent-ops", 7, 40.06],
  ["soap-frontend", 16, 25.5],
  ["t3code", 1, 23.42],
  ["soap-backend", 5, 15.91],
  ["4 others", 7, 11.97],
];

const statCards = tiles([
  { label: "Tasks created", value: "90", sub: "117 touched &middot; 63 closed same day" },
  { label: "Ended in error", value: "0", sub: "of 117 tasks", tone: "good" },
  { label: "Agent messages", value: "1,750", sub: "939 turns &middot; 319.1 M tokens" },
  { label: "Cost, new tasks", value: "$480", sub: "+ $1,271 lifetime on 27 carried over" },
  {
    label: "K8s warning events",
    value: "2,062",
    sub: "1,045 FailedScheduling &middot; 757 Unhealthy",
    tone: "critical",
  },
  { label: "Pods per sandbox", value: "346 / 116", sub: "52 sandboxes used more than one pod", tone: "critical" },
  { label: "Backend errors", value: "50", sub: "34 user-facing 5xx &middot; 0 panics" },
  { label: "Feedback filed", value: "31", sub: "all by agents &middot; 26 still open" },
]);

const tasksPerHour = card({
  heading: "Tasks per hour",
  right: "90 created &middot; 117 touched &middot; UTC",
  note: `Stacked: new tasks, and tasks created earlier that were still being worked.
            Creation stays flat all day — the load spikes below are not a rush of new work.`,
  body: columns({
    labels: HOURS,
    tips: CLOCK,
    series: [
      { name: "created", values: CREATED },
      { name: "carried over", values: CARRIED },
    ],
    peaks: 2,
  }),
});

const warningEvents = card({
  heading: "Kubernetes warning events per hour",
  right: "2,062 events &middot; UTC",
  note: "Stacked; hover a column for the hour&rsquo;s breakdown.",
  body:
    columns({
      labels: HOURS,
      tips: CLOCK,
      series: Object.entries(WARNINGS).map(([name, values]) => ({ name, values })),
      peaks: 2,
    }) +
    `
          <div class="mt-5 rule pt-3">
${table({
  cols: ["Reason", { h: "Total", n: true }, { h: "Peak hour", n: true }, "Shape of the day"],
  rows: SHAPE.map(([reason, peak, shape], i) => [
    `${swatch(i)}${reason}`,
    num(WARNINGS[reason].reduce((a, b) => a + b, 0)),
    peak,
    `<span class="ink2">${shape}</span>`,
  ]),
})}
            <p class="text-xs muted mt-2">
              Not charted: 136 <code>TaintManagerEviction</code> (same 04:00&ndash;10:00 window as
              NodeNotReady), 336 <code>Killing</code>, 29 <code>Evicted</code>. Zero OOMKills and
              zero image-pull failures all day.
            </p>
          </div>`,
});

const loopsFired = card({
  heading: "Loops that fired",
  body:
    table({
      cols: ["Loop", "Kind", { h: "Tasks", n: true }],
      rows: LOOPS.map(([name, kind, n]) => [name, `<span class="ink2">${kind}</span>`, num(n)]),
      foot: ["12 loops fired", `<span class="ink2">of 34 active</span>`, num(70)],
    }) +
    `
          <p class="text-xs muted mt-3">
            Silent but active: all five <code>alerts-*</code> monitors, <code>e2e-triage</code>,
            <code>release-train</code>, <code>cost-anomaly-investigation</code>,
            <code>test-improver</code>, <code>docs-improver</code> and 12 more. Five backend
            rollouts shipped during the day, at 09:43, 10:10, 12:43, 14:29 and 15:26.
          </p>`,
});

const costByRepo = card({
  heading: "Cost by repository",
  note: "Tasks created 2026-09-16 &middot; $480.39 total",
  body:
    hbars(
      COST.map(([repo, tasks, usd]) => ({
        label: `${repo} <span class="muted tnum">(${tasks})</span>`,
        value: usd,
        text: `$${usd.toFixed(2)}`,
      })),
    ) +
    `
          <p class="text-xs muted mt-4">
            t3code is one task at $23.42 — the upstream merge that lost six verification
            passes to eviction. soap-frontend spent $25.50 across 16 tasks.
          </p>`,
});

await writeFile(
  OUT,
  page({
    title: "System health, 2026-09-16 — skill reference render",
    env: "daily-health-report &middot; reference render &middot; namespace <code>moatless</code>",
    heading: "System health, 2026-09-16",
    lede: `Sections 2 through 5 of that day&rsquo;s report, rebuilt from <code>layout.mjs</code>:
        the tiles, four cards of <strong>Tasks, metrics and cost</strong>, and the findings and
        feedback the original page carried. <em>What to do with this</em> and
        <em>Method and caveats</em> are left off — they are prose, and the components are what
        this page is for.`,
    sources: "Loki <code>namespace=&quot;moatless&quot;</code> &middot; moatless REST API &middot; all times UTC",
    body: [
      statCards,
      section({
        heading: "Tasks, metrics and cost",
        body: [tasksPerHour, warningEvents, loopsFired, costByRepo].join("\n"),
      }),
      section({ heading: "Findings", space: 8, body: findingsHtml }),
      section({ heading: "Feedback report", body: feedbackHtml }),
    ].join("\n"),
    footer: "Rendered by <code>scripts/example.mjs</code> from the daily-health-report skill.",
  }),
);
console.log(`wrote ${OUT}`);

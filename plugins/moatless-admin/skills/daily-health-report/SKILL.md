---
name: daily-health-report
description: Build the daily Moatless environment health report — collect one UTC day of task, Loki and Kubernetes data with the bundled collector, then render a self-contained HTML page with stat tiles, a metrics section, the three biggest findings and a feedback breakdown. Use when asked for a health report, a daily or yesterday's report on the environment, a system-health snapshot of the moatless namespace, or when the daily-health-check loop fires. Not for diagnosing one stuck sandbox (use sandbox-troubleshooting) and not for right-sizing workspace CPU or memory (use workspace-resource-tuning).
---

# Daily health report

One HTML page covering one UTC day of the `moatless` namespace, written for a person to read: what the platform did, what it did badly, and the three things worth acting on.

## Workflow

**1. Collect the day.** The collector runs every check and writes both the raw JSON and the aggregates the tiles need.

```bash
export GRAFANA_TOKEN=…                     # already in .envrc; never print it
mkdir -p scratchpad/data && cd scratchpad
node <skill>/scripts/collect.mjs 2026-09-16 --out data
```

It writes `data/digest.json` plus `tasks`, `feedback`, `loops`, `workspaces`, `repos`, `stats`, `k8s_events`, `sandbox_pods`, `log_levels`, `http_5xx` and `patterns`. `--only api|stats|loki` re-runs one group after a failure and merges into the digest already on disk, so the other groups survive. The stats group owns `tasks.touched`, `tasks.error`, `tasks.costTouchedUsd` and `tasks.carriedPerHour` — an `--only api` run leaves whatever a previous full run put there. Loki takes seconds; the API group takes a few minutes for yesterday and longer the further back the date is, because every task still alive after that date is a candidate and each one costs a `/statistics` call.

**2. Read the digest before writing anything.** It carries every tile number, the per-reason Kubernetes warning breakdown, the log-level totals per service and the tool-failure rate. Anything the digest does not answer needs a query of its own — `references/queries.md` has the ones that already work.

**3. Pick the three findings.** Three, not four, and not one per data source. See *Picking the three* below.

**4. Build the page.** Write a `build.mjs` beside the data that imports `scripts/layout.mjs`, feeds it the digest, and writes the HTML. Sections the library has no component for are plain HTML strings in the same file — the library imposes the shell, the CSS and the chart geometry, nothing else.

```js
import { page, section, tiles, card, columns, hbars, table, swatch, num } from "<skill>/scripts/layout.mjs";
const d = JSON.parse(await readFile("data/digest.json", "utf8"));

const metrics = section({ heading: "Tasks, metrics and cost", body: [
  card({ heading: "Agent messages per hour", right: `${num(d.agents.messages)} total · UTC`,
         body: columns({ labels: HOURS, series: [{ name: "messages", values: d.agents.msgsPerHour }] }) }),
  card({ heading: "Kubernetes warning events per hour", right: `${num(d.k8sWarnings.total)} events · UTC`,
         body: columns({ labels: HOURS, series: k8sSeries, peaks: 2 }) }),
].join("\n") });
```

**5. Validate and hand it over.** Check tag balance before anyone opens it:

```bash
python3 -c "
from html.parser import HTMLParser
class P(HTMLParser):
    def __init__(s): super().__init__(); s.st=[]; s.bad=[]
    def handle_starttag(s,t,a):
        if t not in ('meta','br','hr','img','input','link'): s.st.append(t)
    def handle_endtag(s,t):
        s.st.pop() if s.st and s.st[-1]==t else s.bad.append(t)
p=P(); p.feed(open('$FILE').read()); print(p.bad, p.st)"
```

A clean page prints the two self-closed `<meta />` tags as mismatches and nothing unclosed. Anything else is a real imbalance, almost always an unescaped `<` in a log line or a repo name — pass it through `esc()`.

Give the reader the workspace-relative path once — `scratchpad/moatless-health-<date>-<timestamp>.html` — and the three findings in two sentences each.

## Section order

Fixed. A reader scanning the page should reach the findings having already seen the numbers behind them.

| # | Section | Contents |
|---|---|---|
| 1 | Header | Environment kicker, `System health, <date>`, a lede naming the day's one big fact, a sources line |
| 2 | Stat tiles | Eight tiles, no heading — the grid is four columns, so use four or eight |
| 3 | `Tasks, metrics and cost` | Heading only, then cards. No intro prose |
| 4 | `Findings` | The heading, then three findings as `h3` |
| 5 | `Feedback report` | Category split, then clusters |
| 6 | `What to do with this` | Ordered by payoff, each naming who acts |
| 7 | `Method and caveats` | Sources, windows, what was not verified |

**Write the section's name as its heading.** No kicker line above it, and no descriptive headline in its place: `Feedback report`, not *31 items carrying 40 distinct problems*. The one exception is the page header's environment line, which is context rather than a section label.

The eight tiles: tasks created, ended in error, agent messages, cost for tasks created that day, Kubernetes warning events, pods per sandbox, backend errors, feedback filed. Colour a tile only when its value is a problem — `--critical` for a number someone must act on, `--good` for a zero that matters.

Section 3 carries the measurements, including every one a finding cites. It opens with **Tasks per hour**, then **Kubernetes warning events per hour** stacked by reason, then **Loops that fired** — that order puts the day's workload in front of the cluster's reaction to it. After those: agent messages per hour, sandbox pods per hour, cost by repository, tool calls and their failure rates, warn/error log volume by service, HTTP 5xx by path, and a **comparison table against the previous day**, built from that day's `digest.json` rather than from its published page so both columns are measured the same way.

**Loops that fired** comes from `bindings[].loopId` on each created task, joined to `loops.json`. Only adapter-event loops set it, so the table covers those and the note under it accounts for the unattributed remainder — the scheduled loops' runs and the tasks people started by hand.

`scripts/example.mjs` renders sections 2 through 5 from 2026-09-16's real numbers, with no data directory and no network. Run it and read it beside the source when writing a new `build.mjs`. Its findings and feedback come from `example-findings.html` and `example-feedback.html`, read off disk rather than inlined — that is the pattern for any section no component draws, and it keeps the prose out of a template literal that would otherwise have to escape it.

## Picking the three

Rank by what a reader would act on, not by what produced the most lines.

- **A number nobody can act on is not a finding.** 2.7 M harmless WARN lines is a metric; *four patterns are 88% of ingest and here is the one to delete first* is a finding.
- **Prefer a failure that reached a person or destroyed work** over one that cost only time.
- **Fold repeats into one finding.** Six evictions with the same cause are one finding with six instances, not six findings.
- **Name the ones you dropped** in the method note, so the next reader can tell a quiet area from an unexamined one.

Write each finding as three paragraphs: what happened and what it cost, the evidence with counts and timestamps, then what to do. A reader who stops after the first paragraph should still have the finding. Where the cause is inferred rather than measured, say so once — *may* — and keep the observation separate from the inference.

## Gotchas

These defeat a reasonable assumption, and each one has produced a wrong number here before.

- **A `count_over_time(...[1h])` sample at `t` covers `(t-1h, t]`.** Query `start` at the day's 01:00 and `end` at the next day's 00:00, and label each sample with the hour *before* it. Querying 00:00–24:00 gives you 25 buckets whose first is the previous day.
- **The Kubernetes event stream carries `Normal` events too**, and they outnumber the warnings roughly four to one. Extract `type` and filter `etype="Warning"` — an unfiltered count reads as a catastrophe.
- **`| json | status >= 500` fails with `LabelFilterErr`.** Lines without an HTTP status parse to `""`, which is not a float. Match the string instead: `| status =~ "5[0-9][0-9]"`.
- **Datasource uid `loki` with `namespace="moatless"` is the moatless staging namespace.** The `soap` namespace on the same datasource is a different product; `-prod` datasources are a different environment entirely.
- **A sandbox pod name encodes the task id, not the workspace id.** Join through `moat tasks list --json` before attributing pod behaviour to a workspace.
- **`status` and `totalCostUsd` are read at collection time.** A task that closes an hour later changes the answer, so date the collection in the method note rather than presenting these as the day's final state.
- **`lastMessageAt` moves forward for as long as a task lives.** Counting a task as touched when its *last* message falls inside the window drops it the moment it speaks again the next day, so the same date collected twice gives two different answers. Take every task whose last message is at or after the window opens, and let the per-message timestamps in `statistics` decide which of them ran in it.
- **`messageTokenUsage` records agent turns only.** A task whose only activity in the window was a person writing to it has no rows there, so fall back to `lastMessageAt` before concluding it was idle.
- **A task row keys on `taskId`, not `id`**, and carries `bindings[].loopId` for adapter-event loops only.
- **`toolCalls` in `/statistics` is a lifetime total with no timestamps.** The collector tallies it over the tasks that ran on the day, so a task that resumed brings its earlier days' calls with it. Read the tool table as "the tasks that worked that day, over their whole lives", and do not put a per-day rate on it.
- **The collector stops at `CAP` (5,000) rows** and warns only when the oldest row it pulled is newer than the day being collected. A warning means the day is truncated and `CAP` has to rise; hitting the cap silently is normal.
- **Node writes to disk emit no viewer event.** The first build is fine because the tab is not open yet; any later revision must go through the Edit tool, or the reader's open tab silently keeps the old page.
- **Never print `GRAFANA_TOKEN`**, in a log line, a command echo, or the page.

## Done when

- Every number on the page traces to a file in `data/` or to a query written down in the report's method note.
- Three findings, each with consequence, evidence and an action, and each citing a chart that is on the page.
- No kicker above any section heading, and no numbered *Finding N* labels.
- The tag-balance check reports nothing unclosed and nothing mismatched beyond the two `<meta />` tags.
- Charts carry a legend or a table when they hold more than one series, and every tooltip names its hour.

## References

- `references/queries.md` — read when the digest does not answer a question, or a query returns empty. Holds every LogQL and REST call the collector makes, with the shape each returns.
- `references/layout.md` — read while writing `build.mjs`. The full component API, and what a custom section must match to sit beside the built-in ones.

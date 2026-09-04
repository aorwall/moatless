#!/usr/bin/env python3
"""Reprice historical task cost from the S3 agent transcripts.

`tasks.total_cost_usd` counted only what the backend recorded at the time, and
before the run-totals fix it recorded nothing for most runs. The transcripts in
S3 hold every API request each task ever made, so they are the authority for
what a task cost.

Three subcommands, run in order:

    scan    read every task's transcripts.tar from S3 into a JSONL file
    report  join that file against the database and print the difference
    apply   write the scanned figures into tasks.total_cost_usd

`scan` is resumable. It appends one JSON object per task and skips a task the
output file already holds, so an interrupted run continues where it stopped.

`apply` writes to production and refuses to run without --yes.
"""

import argparse
import json
import os
import subprocess
import sys
import tarfile
import tempfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

BUCKET = os.environ.get("TRANSCRIPT_BUCKET", "soap-fp-staging-moatless-uploads")
PREFIX = os.environ.get("TRANSCRIPT_PREFIX", "transcripts")

# USD per 1M tokens. Claude splits cache writes by TTL; the 1h tier costs more.
#
# Derived from Anthropic's own per-model `costUSD` in `task_turns.metrics ->
# modelUsage`, which the run-totals fix started recording: each rate below
# reproduces every ground-truth row for its model. Tokens above the 200k
# context mark price at the same rate. The 1M-context models carry no
# long-context premium, so do not add a tier multiplier for them.
#
# Matched in order, so a specific generation precedes the family name it
# contains: `sonnet-5` costs two thirds of `sonnet-4-6`, and a match on plain
# "sonnet" first would bill it as the older, dearer model.
CLAUDE_PRICES = {
    "opus": dict(input=5.0, cw5m=6.25, cw1h=10.0, cache_read=0.5, output=25.0),
    "fable": dict(input=10.0, cw5m=12.5, cw1h=20.0, cache_read=1.0, output=50.0),
    "sonnet-5": dict(input=2.0, cw5m=2.5, cw1h=4.0, cache_read=0.2, output=10.0),
    "sonnet": dict(input=3.0, cw5m=3.75, cw1h=6.0, cache_read=0.3, output=15.0),
    "haiku": dict(input=1.0, cw5m=1.25, cw1h=2.0, cache_read=0.1, output=5.0),
}
# OpenAI bills a cached input token at a tenth of a fresh one and does not bill
# the cache write separately. The cache-write field is priced at the input rate
# here because some rollouts report it non-zero.
CODEX_PRICE = dict(input=1.25, cached=0.125, cache_write=1.25, output=10.0)


def claude_price(model):
    """Prices for the model, or `None` where the run is not billable.

    `<synthetic>` is the model on a message the CLI injected rather than
    bought, such as an API-error notice. It reaches no API and costs nothing,
    so it prices as `None` and lands in the `claude_unpriced` count.
    """
    name = (model or "").lower()
    for key, price in CLAUDE_PRICES.items():
        if key in name:
            return price
    return None


def scan_claude(root):
    """Cost and tokens for every Claude session in one task.

    Deduplicates on `requestId`: the CLI writes the same assistant message
    several times, for streaming snapshots and for resume replays, so a raw row
    sum counts one API request many times. Sub-agent rows carry their own
    request ids and their own real spend, so this counts them.

    Of the rows that share a `requestId` it keeps the one with the largest
    `output_tokens`, never the first and never their sum. The rows of one
    request repeat the input and cache counts identically while `output_tokens`
    grows to the response's final size. The first row is therefore a partial
    snapshot, and the sum counts those partials again. Sub-agent requests are
    the ones that split this way, and output is the dearest tier, so a match on
    the first row understated them by more than an order of magnitude.
    """
    seen = {}
    for path in sorted(root.glob("claude/**/*.jsonl")):
        for line in path.open(errors="replace"):
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get("type") != "assistant":
                continue
            message = row.get("message") or {}
            usage = message.get("usage")
            if not usage:
                continue
            key = row.get("requestId") or f"uuid:{row.get('uuid')}"
            prior = seen.get(key)
            if prior is not None and int(prior[1].get("output_tokens") or 0) >= int(
                usage.get("output_tokens") or 0
            ):
                continue
            seen[key] = (message.get("model"), usage)

    cost = 0.0
    totals = dict(input=0, output=0, cache_write=0, cache_read=0)
    by_model = {}
    unpriced = 0
    for model, usage in seen.values():
        price = claude_price(model)
        creation = usage.get("cache_creation") or {}
        cw5m = int(creation.get("ephemeral_5m_input_tokens") or 0)
        cw1h = int(creation.get("ephemeral_1h_input_tokens") or 0)
        if not (cw5m or cw1h):
            cw5m = int(usage.get("cache_creation_input_tokens") or 0)
        inp = int(usage.get("input_tokens") or 0)
        out = int(usage.get("output_tokens") or 0)
        read = int(usage.get("cache_read_input_tokens") or 0)
        totals["input"] += inp
        totals["output"] += out
        totals["cache_write"] += cw5m + cw1h
        totals["cache_read"] += read
        bucket = by_model.setdefault(
            model or "<none>",
            dict(input=0, cw5m=0, cw1h=0, cache_read=0, output=0, requests=0),
        )
        bucket["input"] += inp
        bucket["cw5m"] += cw5m
        bucket["cw1h"] += cw1h
        bucket["cache_read"] += read
        bucket["output"] += out
        bucket["requests"] += 1
        if price is None:
            unpriced += 1
            continue
        cost += (
            inp * price["input"]
            + cw5m * price["cw5m"]
            + cw1h * price["cw1h"]
            + read * price["cache_read"]
            + out * price["output"]
        ) / 1_000_000.0
    return cost, totals, len(seen), unpriced, by_model


def scan_codex(root):
    """Cost and tokens for every Codex rollout in one task.

    `total_token_usage` is cumulative within one rollout file, so the last
    reading is that session's whole spend. A resume opens a NEW rollout file
    with its own counter that starts from zero, so a sum of the last reading of
    each file is correct and does not double count.

    Codex reports `input_tokens` gross, which includes the cached prefix, so
    this subtracts `cached_input_tokens` to get the fresh input.
    """
    cost = 0.0
    totals = dict(input=0, output=0, cache_write=0, cache_read=0)
    sessions = 0
    for path in sorted(root.glob("codex/**/*.jsonl")):
        last = None
        for line in path.open(errors="replace"):
            if "token_count" not in line:
                continue
            try:
                row = json.loads(line)
            except ValueError:
                continue
            payload = row.get("payload") or {}
            if payload.get("type") != "token_count":
                continue
            info = (payload.get("info") or {}).get("total_token_usage")
            if info:
                last = info
        if not last:
            continue
        sessions += 1
        gross = int(last.get("input_tokens") or 0)
        cached = int(last.get("cached_input_tokens") or 0)
        net = max(gross - cached, 0)
        out = int(last.get("output_tokens") or 0)
        write = int(last.get("cache_creation_input_tokens") or 0)
        totals["input"] += net
        totals["output"] += out
        totals["cache_write"] += write
        totals["cache_read"] += cached
        cost += (
            net * CODEX_PRICE["input"]
            + cached * CODEX_PRICE["cached"]
            + write * CODEX_PRICE["cache_write"]
            + out * CODEX_PRICE["output"]
        ) / 1_000_000.0
    return cost, totals, sessions


def scan_task(task_id):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        tar_path = root / "transcripts.tar"
        key = f"s3://{BUCKET}/{PREFIX}/{task_id}/transcripts.tar"
        got = subprocess.run(
            ["aws", "s3", "cp", key, str(tar_path), "--quiet"],
            capture_output=True, text=True,
        )
        if got.returncode != 0:
            return dict(task_id=task_id, error=f"s3: {got.stderr.strip()}"[:200])
        try:
            with tarfile.open(tar_path) as tar:
                tar.extractall(root, filter="data")
        except Exception as exc:
            return dict(task_id=task_id, error=f"tar: {exc}"[:200])

        c_cost, c_tok, c_reqs, c_unpriced, c_by_model = scan_claude(root)
        x_cost, x_tok, x_sessions = scan_codex(root)
        return dict(
            task_id=task_id,
            cost=round(c_cost + x_cost, 6),
            claude_cost=round(c_cost, 6),
            codex_cost=round(x_cost, 6),
            claude_requests=c_reqs,
            claude_unpriced=c_unpriced,
            codex_sessions=x_sessions,
            tokens={k: c_tok[k] + x_tok[k] for k in c_tok},
            # Per-model, per-cache-tier tokens, so a correction to a rate in
            # CLAUDE_PRICES reprices from this file instead of a second
            # download of every tar from S3.
            claude_by_model=c_by_model,
        )


def list_task_ids():
    out = subprocess.run(
        ["aws", "s3", "ls", f"s3://{BUCKET}/{PREFIX}/"],
        capture_output=True, text=True, check=True,
    ).stdout
    return [
        ln.split()[-1].rstrip("/")
        for ln in out.splitlines()
        if ln.strip().startswith("PRE")
    ]


def cmd_scan(args):
    done = set()
    out_path = Path(args.out)
    if out_path.exists():
        for line in out_path.open():
            try:
                done.add(json.loads(line)["task_id"])
            except (ValueError, KeyError):
                pass
    ids = [t for t in list_task_ids() if t not in done]
    print(f"{len(done)} already scanned, {len(ids)} to go", file=sys.stderr)
    if args.limit:
        ids = ids[: args.limit]

    written = 0
    with out_path.open("a") as sink, ThreadPoolExecutor(args.workers) as pool:
        futures = {pool.submit(scan_task, t): t for t in ids}
        for future in as_completed(futures):
            sink.write(json.dumps(future.result()) + "\n")
            sink.flush()
            written += 1
            if written % 100 == 0:
                print(f"  {written}/{len(ids)}", file=sys.stderr)
    print(f"scanned {written} tasks into {out_path}", file=sys.stderr)


def load_scan(path):
    rows = {}
    for line in Path(path).open():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if "error" not in row:
            rows[row["task_id"]] = row
    return rows


def db_costs(dburl, task_ids):
    ids = ",".join("'" + t.replace("'", "") + "'" for t in task_ids)
    sql = f"SELECT task_id, total_cost_usd FROM tasks WHERE task_id IN ({ids});"
    out = subprocess.run(
        ["psql", dburl, "-X", "-q", "-t", "-A", "-F", "\t", "-c", sql],
        capture_output=True, text=True, check=True,
    ).stdout
    stored = {}
    for line in out.splitlines():
        if "\t" in line:
            tid, val = line.split("\t", 1)
            stored[tid] = float(val)
    return stored


def cmd_report(args):
    scan = load_scan(args.scan)
    stored = db_costs(args.dburl, list(scan))
    rows = []
    for tid, row in scan.items():
        if tid not in stored:
            continue
        rows.append((tid, stored[tid], row["cost"], row["cost"] - stored[tid], row))
    rows.sort(key=lambda r: -abs(r[3]))

    print(f"{'task_id':38} {'stored':>10} {'transcript':>11} {'delta':>11}")
    for tid, was, now, delta, _ in rows[: args.top]:
        print(f"{tid:38} {was:10.2f} {now:11.2f} {delta:+11.2f}")

    tot_was = sum(r[1] for r in rows)
    tot_now = sum(r[2] for r in rows)
    zeroed = [r for r in rows if r[1] == 0 and r[2] > 0]
    lower = [r for r in rows if r[3] < -0.005]
    print(f"\ntasks in both S3 and DB : {len(rows)}")
    print(f"stored total            : ${tot_was:,.2f}")
    print(f"transcript total        : ${tot_now:,.2f}")
    print(f"net change              : ${tot_now - tot_was:+,.2f}")
    print(f"tasks now $0 that gain  : {len(zeroed)} (${sum(r[2] for r in zeroed):,.2f})")
    print(f"tasks that go DOWN      : {len(lower)} (${sum(r[3] for r in lower):,.2f})")
    print(f"missing from DB         : {len(scan) - len(rows)}")


def cmd_apply(args):
    if not args.yes:
        sys.exit("refusing to write without --yes")
    scan = load_scan(args.scan)
    stored = db_costs(args.dburl, list(scan))
    pairs = [(t, r["cost"]) for t, r in scan.items() if t in stored]
    if not pairs:
        sys.exit("nothing to write")
    values = ",".join(f"('{t}',{c:.6f}::float8)" for t, c in pairs)
    sql = (
        f"UPDATE tasks SET total_cost_usd = v.cost, updated_at = NOW() "
        f"FROM (VALUES {values}) AS v(task_id, cost) "
        f"WHERE tasks.task_id = v.task_id;"
    )
    out = subprocess.run(
        ["psql", args.dburl, "-X", "-q", "-v", "ON_ERROR_STOP=1", "-c", sql],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        sys.exit(out.stderr.strip())
    print(f"updated {len(pairs)} tasks")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("scan", help="read transcripts from S3 into a JSONL file")
    p.add_argument("--out", default="/tmp/task-costs.jsonl")
    p.add_argument("--workers", type=int, default=16)
    p.add_argument("--limit", type=int, default=0)
    p.set_defaults(func=cmd_scan)

    p = sub.add_parser("report", help="print the difference against the database")
    p.add_argument("--scan", default="/tmp/task-costs.jsonl")
    p.add_argument("--dburl", default=os.environ.get("DATABASE_URL", ""))
    p.add_argument("--top", type=int, default=25)
    p.set_defaults(func=cmd_report)

    p = sub.add_parser("apply", help="write the scanned figures to the database")
    p.add_argument("--scan", default="/tmp/task-costs.jsonl")
    p.add_argument("--dburl", default=os.environ.get("DATABASE_URL", ""))
    p.add_argument("--yes", action="store_true")
    p.set_defaults(func=cmd_apply)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()

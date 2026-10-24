#!/usr/bin/env python3
"""Agent side of the user-admin page: publish, plan, apply.

    user_admin.py publish [--out DIR]       snapshot the directory and publish
    user_admin.py plan <request-id>         check a request, save plans/<id>
    user_admin.py apply <request-id>        apply an approved plan, save results/<id>

`assets/user-admin-rules.json` lets readers only create `requests/` and
`approvals/` records, once each, and leaves `plans/` and `results/` to the
agent. Who may approve is not expressible in the rules, so `apply` checks the
approver, and checks the request against the live directory again.
"""
import argparse
import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path

SLUG = "user-admin"
TITLE = "User admin"
ASSETS = Path(__file__).resolve().parent.parent / "assets"
TEMPLATE = ASSETS / "user-admin.html"
RULES = ASSETS / "user-admin-rules.json"
LOG_KEY = "admin:changes"
FIELDS = {"name", "email", "role", "status"}
ROLES = {"admin", "user"}
STATUSES = {"active", "disabled"}
EMAIL = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")
MAX_CHANGES = 20


def moat(*args, check=True):
    # Argument list, never a shell string: record values reach these calls.
    proc = subprocess.run(["moat", *args], capture_output=True, text=True)
    if check and proc.returncode != 0:
        raise RuntimeError(f"moat {args[0]} {args[1]} failed: {proc.stderr.strip()}")
    return proc


def api(method, path, body):
    """`moat api` exits 0 on any HTTP status, so read the status it prints."""
    proc = moat("api", method, path, "--body", json.dumps(body), check=False)
    try:
        reply = json.loads(proc.stdout)
        status = int(reply["status"])
    except (ValueError, KeyError, TypeError):
        return f"unreadable reply: {(proc.stderr or proc.stdout).strip()[:300]}"
    return None if 200 <= status < 300 else f"HTTP {status}: {json.dumps(reply.get('body'))[:300]}"


def live_users():
    out = moat("user", "list", "--include-disabled", "--json").stdout
    return {u["id"]: u for u in json.loads(out)["users"]}


def get_record(key):
    proc = moat("artifact", "data", "get", SLUG, key, "--json", check=False)
    if proc.returncode != 0 or proc.stdout.strip() in ("", "null"):
        return None
    return json.loads(proc.stdout)


def save_record(key, data):
    moat("artifact", "data", "save", SLUG, key, json.dumps(data))


def now():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def active_admin(users, user_id):
    u = users.get(user_id)
    return u is not None and u["role"] == "admin" and u["status"] == "active"


def check_value(field, to):
    if field == "name":
        if to is not None and (not isinstance(to, str) or len(to) > 200
                               or any(ord(c) < 32 for c in to)):
            return "name must be at most 200 printable characters"
    elif field == "email":
        if not isinstance(to, str) or not EMAIL.match(to) or len(to) > 254:
            return "not a valid email address"
    elif field == "role" and to not in ROLES:
        return f"role must be one of {sorted(ROLES)}"
    elif field == "status" and to not in STATUSES:
        return f"status must be one of {sorted(STATUSES)}"
    return None


def validate(request, users):
    """Return (problem, checked changes). `problem` rejects the whole request."""
    submitter = request.get("submittedBy")
    if not active_admin(users, submitter):
        return "submitter is not an active admin", []
    changes = (request.get("data") or {}).get("changes")
    if not isinstance(changes, list) or not changes:
        return "request has no changes", []
    if len(changes) > MAX_CHANGES:
        return f"more than {MAX_CHANGES} changes in one request", []

    checked = []
    admins_after = {i for i in users if active_admin(users, i)}
    for c in changes:
        c = {k: c.get(k) for k in ("userId", "field", "from", "to")}
        target = users.get(c["userId"])
        if c["field"] == "name" and c["to"] == "":
            c["to"] = None
        if target is None:
            reason = "no such user"
        elif c["field"] not in FIELDS:
            reason = "field cannot be changed here"
        elif target[c["field"]] != c["from"]:
            reason = f"changed since the page loaded; it is now {target[c['field']]!r}"
        elif c["from"] == c["to"]:
            reason = "no change"
        else:
            reason = check_value(c["field"], c["to"])
        if reason is None and c["userId"] == submitter and c["field"] in {"role", "status"}:
            reason = "you cannot change your own role or status"
        if reason is None and (c["field"], c["to"]) in {("role", "user"), ("status", "disabled")}:
            admins_after.discard(c["userId"])
        c["login"] = target["login"] if target else None
        c["ok"] = reason is None
        c["reason"] = reason
        checked.append(c)
    if not admins_after:
        return "the request would leave no active admin", checked
    return None, checked


def log_change(line):
    if moat("memory", "get", LOG_KEY, check=False).returncode != 0:
        moat("memory", "put", LOG_KEY, "--summary",
             "Log of admin changes made to the Moatless deployment", "--text", "")
    moat("memory", "append", LOG_KEY, "--text", line)


def cmd_plan(args):
    users = live_users()
    request = get_record(f"requests/{args.id}")
    if request is None:
        raise SystemExit(f"no record requests/{args.id}")
    problem, checked = validate(request, users)
    status = "rejected" if problem or not any(c["ok"] for c in checked) else "ready"
    plan = {"status": status, "reason": problem, "changes": checked, "plannedAt": now()}
    save_record(f"plans/{args.id}", plan)
    print(json.dumps(plan, indent=2, ensure_ascii=False))


def cmd_apply(args):
    users = live_users()
    approval = get_record(f"approvals/{args.id}")
    if approval is None or not active_admin(users, approval.get("submittedBy")):
        raise SystemExit("approval missing or not from an active admin")
    request = get_record(f"requests/{args.id}")
    if request is None:
        raise SystemExit(f"no record requests/{args.id}")
    if args.distinct_approver and approval["submittedBy"] == request["submittedBy"]:
        raise SystemExit("the submitter cannot approve their own request")
    plan = get_record(f"plans/{args.id}")
    if plan is None or plan["data"]["status"] != "ready":
        raise SystemExit("no ready plan to approve")
    if get_record(f"results/{args.id}") is not None:
        raise SystemExit("already applied")
    problem, checked = validate(request, users)
    if problem:
        save_record(f"results/{args.id}", {"status": "rejected", "reason": problem,
                                           "changes": checked, "appliedAt": now()})
        raise SystemExit(problem)

    # Checking again may only drop a change: one the approver saw rejected
    # stays rejected even if it would pass now.
    approved = {(c["userId"], c["field"], c["to"]) for c in plan["data"]["changes"] if c["ok"]}
    by_user = {}
    for c in checked:
        if c["ok"] and (c["userId"], c["field"], c["to"]) not in approved:
            c["ok"], c["reason"] = False, "not in the approved plan"
        if c["ok"]:
            by_user.setdefault(c["userId"], {})[c["field"]] = c["to"]
    errors = {}
    for user_id, body in by_user.items():
        error = api("PATCH", f"/api/v1/admin/users/{user_id}", body)
        if error:
            errors[user_id] = error
    for c in checked:
        if c["ok"] and c["userId"] in errors:
            c["ok"], c["reason"] = False, errors[c["userId"]]
        elif c["ok"]:
            c["reason"] = "applied"
    applied = sum(c["reason"] == "applied" for c in checked)
    status = "applied" if applied == len(approved) else ("partial" if applied else "failed")
    result = {"status": status, "changes": checked, "appliedAt": now()}
    save_record(f"results/{args.id}", result)
    login = lambda i: users.get(i, {}).get("login", i)
    log_change(f"request {args.id} by {login(request['submittedBy'])}, approved by "
               f"{login(approval['submittedBy'])}: " + "; ".join(
                   f"{c['login']}.{c['field']} {c['from']!r}->{c['to']!r} {c['reason']}"
                   for c in checked))
    publish(args.out)
    print(json.dumps(result, indent=2, ensure_ascii=False))


def publish(out):
    users = sorted(live_users().values(), key=lambda u: (u["isBot"], u["login"].lower()))
    snapshot = {"takenAt": now(), "users": users}
    # `</` inside a JSON string would end the <script> block early.
    data = json.dumps(snapshot, ensure_ascii=False).replace("</", "<\\/")
    out.mkdir(parents=True, exist_ok=True)
    page = out / f"{SLUG}.html"
    page.write_text(TEMPLATE.read_text().replace("__SNAPSHOT__", data))
    # --private: the page lists every user's email. --rules again on every
    # publish, or the page loses them.
    print(moat("artifact", "publish", str(page), "--title", TITLE, "--slug", SLUG,
               "--private", "--data", "--rules", str(RULES)).stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path("scratchpad/user-admin"),
                        help="where the rendered page is written")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("publish").set_defaults(run=lambda a: publish(a.out))
    for name, run in (("plan", cmd_plan), ("apply", cmd_apply)):
        p = sub.add_parser(name)
        p.add_argument("id", help="the request id, the part of requests/<id> after the slash")
        if name == "apply":
            p.add_argument("--distinct-approver", action="store_true",
                           help="refuse an approval from the request's own submitter")
        p.set_defaults(run=run)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()

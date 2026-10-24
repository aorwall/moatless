# Users

## Direct changes

| Change | Command |
|---|---|
| Read the directory | `moat user list --include-disabled --json` |
| Display name | `moat user set-name <user> "<name>"` |
| Email | `moat user set-email <user> <email>` |
| Deactivate, reactivate | `moat user deactivate <user>`, `moat user activate <user>` |
| Role (`admin` or `user`) | `moat api PATCH /api/v1/admin/users/<id> --body '{"role":"admin"}'` |

`<user>` takes a login or an id. One login can belong to two accounts from
different providers, and `deactivate <login>` then acts on the older one. Pass
the id whenever `user list` shows the login twice.

Refuse a change that leaves no active admin, or that changes the requester's
own role or status. Either can lock everyone out of the admin API.

A deactivated account cannot sign in, and its sessions and API keys stop
working.

## The user admin page

`scripts/user_admin.py` and `assets/user-admin.html` are a finished page for
users. It shows a summary of counts, then the directory with each account's
last sign-in, with accounts not seen in 90 days highlighted. A reader edits
names, emails, roles and statuses and submits them as one request.

```bash
python3 <skill>/scripts/user_admin.py publish            # snapshot the directory, publish
python3 <skill>/scripts/user_admin.py plan <request-id>  # on a requests/<id> notification
python3 <skill>/scripts/user_admin.py apply <request-id> # on an approvals/<id> notification
```

`plan` and `apply` reject a request whose submitter or approver is not an
active admin, a change whose `from` no longer matches the directory, and the
lock-out changes above. `apply` applies only changes the approved plan
marked ok, even if a change it rejected would pass now. Any active admin may
approve, the submitter included; when the person wants a second admin to
approve, run `apply --distinct-approver`. `apply` republishes the page and
logs each change to `admin:changes`. `--out <dir>` sets where the rendered
page is written; it defaults to `scratchpad/user-admin`.

To show more numbers, such as each user's task count, fetch them in `publish`,
add them to the snapshot and render them in the template, under
`artifact-design`.

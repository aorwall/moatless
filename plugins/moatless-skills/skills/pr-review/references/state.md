# Review state record

Step 1 restores it; step 9 saves it, on every turn.

## Where it lives

One record per pull request in the `pr-review-state` collection, scope
`repository`, key `<owner>/<repo>/<number>`. Never a file: the next push often
starts a new task in a fresh sandbox, possibly in another Loop or none, and
`/tmp` and untracked files do not reach it. The `repository` scope reaches
every task on the same repository, whatever its Loop or Workspace.

```json
{
  "pr": 1206,
  "reviewed_sha": "4d9e792…",
  "approved": false,
  "open_findings": [
    {"at": "crates/x/src/y.rs:88", "rule": "rust-dev-guidelines: typed errors",
     "severity": "important", "raised": "4d9e792…", "status": "open",
     "thread": "https://github.com/OWNER/REPO/pull/1206#discussion_r123"}
  ],
  "conclusions": [
    "migration 0142 is the next free number on main — checked"
  ],
  "coverage": {"total": 7, "reviewed": 6, "skipped": 1},
  "ci_wait": null,
  "pending": []
}
```

- `status` is `open` or `declined`. A declined nit stays in the list so a later
  turn does not raise it again; its thread stays open for the author.
- `raised` is the head whose review raised the finding: a nit holds approval
  only while `raised` equals the head under review.
- `ci_wait` is `{"sha", "command"}` while a CI wait runs, else `null`.

Rules:

- **Record conclusions, not evidence.** The next turn needs the verdict, not
  the diff or the output that led to it.
- **Read it first, and trust it.** Re-verify an entry only when the new commits
  change its subject.
- **Save the whole state.** A save replaces the record.
- **Keep it short** — about forty lines; drop conclusions the commits made
  irrelevant.

## Restoring

```bash
KEY=OWNER/REPO/NUMBER
if R=$(moat records get pr-review-state "$KEY" --scope repository 2>&1); then jq '.data' <<<"$R"
elif grep -q 'has no record under key' <<<"$R"; then echo "no state record"
else echo "$R" >&2; echo "state unreadable"; fi
```

**State unreadable** — a timeout, a server or auth error — is not a missing
record. Retry once; when it still fails, review nothing and save nothing, since
a fresh record would overwrite the open findings and declined nits. Say so in
the task's reply and end the turn.

### No record

A missing record does not prove you never reviewed the PR: it can have
expired. Look for your own reviews first:

```bash
moat gh api "repos/OWNER/REPO/pulls/NUMBER/reviews?per_page=100" --paginate \
  --jq ".[] | select(.user.login == \"$ME\") | {commit_id, state}" | tail -n 1
```

No output means an initial review. Otherwise rebuild the state: `reviewed_sha`
is that review's `commit_id`, `approved` is `state == "APPROVED"`, and
`open_findings` are your unresolved threads (`commands.md` § "Threads"), each
with its severity from its marker and `raised` from its first comment's
`originalCommit`. Say in the report that the state was rebuilt.

### The reviewed head is gone

A force-push can remove `reviewed_sha` from the branch:

```bash
git cat-file -e "$REVIEWED_SHA^{commit}" 2>/dev/null || git fetch -q origin "$REVIEWED_SHA" 2>/dev/null
git merge-base --is-ancestor "$REVIEWED_SHA" HEAD && echo ancestor
```

Without `ancestor`, keep the mode but diff from
`git merge-base origin/<base_ref> HEAD`: wider than needed, and nothing goes
unreviewed. Say in the update that the earlier head was force-pushed away.

## Saving

Write the JSON in a quoted heredoc so quotes and backticks reach the record
unchanged:

```bash
moat records save pr-review-state "$KEY" --scope repository "$(cat <<'JSON'
{"pr": NUMBER, "reviewed_sha": "…", "approved": false, …}
JSON
)" || echo "warning: review state not saved" >&2
```

When a save is refused, say so in the report: the next task rebuilds the
state from the PR.

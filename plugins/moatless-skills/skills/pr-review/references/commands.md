# Command reference

Always type `moat gh …`, never bare `gh …`. `moat gh` proxies a PR comment
through the backend and records what a `moat gh api` POST creates, so your own
comments and reviews are not delivered back to you as somebody else's; what it
does not proxy it passes through to `gh`. Pass `-R OWNER/REPO` on every call:
a Bash call can start outside the checkout, where no repository is inferred.

## Checkout

```bash
git fetch origin <head_ref>:refs/remotes/origin/<head_ref> -f
git checkout -B <head_ref> origin/<head_ref>
```

The explicit refspec is required: the sandbox clone fetches one branch, so a
bare `git fetch origin <head_ref>` writes only `FETCH_HEAD` and the checkout
fails. `-B` picks up a force-push, so the same pair refreshes a re-review. If it
refuses over uncommitted edits, `git stash push -m pr-review` first.

## Inline findings

Post every finding of a turn as one review. `line` is the line in the head's
file and must sit inside a diff hunk, or GitHub answers 422; demote such a
finding to a report line.

```bash
# comments.json: [{"path": "src/a.rs", "line": 42, "side": "RIGHT", "body": "<!-- pr-review severity=nit -->\n**Nit** · …"}]
jq -n --slurpfile c comments.json --arg sha "$HEAD_SHA" \
  '{event: "COMMENT", commit_id: $sha, body: "", comments: $c[0]}' > review-payload.json
moat gh api repos/OWNER/REPO/pulls/NUMBER/reviews --method POST --input review-payload.json
```

The payload goes through `--input`: with `--field`, `comments` is sent as a
string and GitHub answers 422 "is not an array". No `--jq` on the call: the
filter strips the ids `moat gh` records.

## Report

Write the markdown to a file, then post it. `"$(cat …)"` is not re-expanded,
so backticks and quotes stay literal:

```bash
moat gh pr comment NUMBER -R OWNER/REPO --body "$(cat review.md)"
```

## Threads

```bash
moat gh api graphql -f query='
  query($owner:String!,$repo:String!,$number:Int!){
    repository(owner:$owner,name:$repo){ pullRequest(number:$number){
      reviewThreads(first:100){ nodes{ id isResolved
        comments(first:1){ nodes{ author{login} path line body url originalCommit{oid} } } } } } } }' \
  -F owner=OWNER -F repo=REPO -F number=NUMBER

moat gh api graphql -f query='
  mutation($id:ID!){ resolveReviewThread(input:{threadId:$id}){ thread{ id } } }' -F id=THREAD_ID
```

Resolve only threads whose first comment is yours.

## CI

`scripts/ci-wait.sh`, beside this skill's `SKILL.md`, polls the PR until every
check on the reviewed head has finished. It prints one line and exits 0 when
all passed or were skipped, 1 when one failed or was cancelled, and 3 when the
head moved. To read the state once, run it under `timeout 20`: exit 124 means
still pending.

When checks are pending, start it as the turn's last command and end the turn.
The sandbox wakes you when it exits, including at its deadline. Record the
command id as `ci_wait` in the state record first:

```bash
moat cmd run --timeout 2700 "bash <skill dir>/scripts/ci-wait.sh OWNER/REPO NUMBER $HEAD_SHA"
```

On the wake:

- **Exit 0** — approve, with `commit_id` pinned so a later push is not
  approved by accident:

  ```bash
  moat gh api repos/OWNER/REPO/pulls/NUMBER/reviews --method POST \
    --field event='APPROVE' --field commit_id="$HEAD_SHA" \
    --field body="CI passed at ${HEAD_SHA:0:7}; no open findings."
  ```

- **Exit 1** — do not approve. Post one line naming the failed check and one
  line of its log (`moat gh run view <run> --log-failed`). Do not reproduce the
  failure locally.
- **Exit 3** — post nothing: the new head brings its own turn.
- **Deadline** — post nothing more; the report already says approval waits on
  CI, and the next push or an `@` mention runs the approval pass.

Then clear `ci_wait` and save the state record. When a new turn finds a
`ci_wait` for an older head, `moat cmd kill` it before reviewing.

Approving with CI green at the decision is the same call with the report as
the body: `--field body="$(cat review.md)"`.

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

## Approve

Submit an approval only when none of yours stands on the head, and pin it to
the reviewed head so a later push is not approved by accident. `$ME` is your
login (`SKILL.md` § "Is a comment addressed to you?"):

```bash
moat gh api "repos/OWNER/REPO/pulls/NUMBER/reviews?per_page=100" --paginate \
  --jq ".[] | select(.user.login == \"$ME\" and .commit_id == \"$HEAD_SHA\" and .state == \"APPROVED\") | .id" | grep -q . ||
  moat gh api repos/OWNER/REPO/pulls/NUMBER/reviews --method POST \
    --field event='APPROVE' --field commit_id="$HEAD_SHA" --field body="$(cat review.md)"
```

The `--jq` is on the read, never on the POST: `moat gh` records the review from
the POST's full response.

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

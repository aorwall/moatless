# Review mode details

Read the section for the mode `SKILL.md` § "Review modes" picked, and
§ "Early stop" on every re-review.

## Early stop

Decide this at step 3, before you read any code: a turn costs what you read,
so an exit after the analysis saves almost nothing. Stop when the increment
`git diff <reviewed_sha>..HEAD` shows any of:

- **no reviewable files** — every path is generated, vendored, binary or a
  lockfile refresh;
- **only formatting or comment changes** — no changed line alters behaviour;
- **a rebase with no content change** — the PR's own diff is unchanged:

```bash
diff <(git diff "$(git merge-base origin/<base_ref> "$REVIEWED_SHA")" "$REVIEWED_SHA") \
     <(git diff "$(git merge-base origin/<base_ref> HEAD)" HEAD) >/dev/null && echo unchanged
```

**One exception:** an increment that touches a file carrying an open finding of
yours may be the author's answer to it, even when it only adds a comment. Read
that one hunk. When it answers the finding, resolve the thread; when it was the
last finding that held approval, run the approval pass instead of stopping.

When you stop:

1. Post what § "A push while a finding is open" says when a finding of yours
   holds approval; when only nits an earlier turn raised are open, do not stop,
   run the approval pass. When you approved the previous head, do what
   § "Post-approval check" branch 3 says. Otherwise post nothing: an empty
   review still notifies every subscriber.
2. Save the state record with the new `reviewed_sha`, and end the turn.

## Update re-review

Review the new commits, not the pull request. A fix is fresh code: check it
against every rule its subject touches, not only the comment it answers, and
treat the author's framing of a fix as a claim to check.

Do not establish again what you established before: the state record's
`conclusions` stand unless the new commits change their subject. Settle each
question at the cheapest level that does: the state record, the diff hunk, the
file, a search across the repository. Past that, report the claim as
unverified and name what would settle it. At about ten tool calls you are
re-reviewing the whole PR: stop and post what you have.

Then resolve the threads whose issue is gone, post new findings inline, and
post the short update format (`findings.md` § "Update") when something changed
or a finding of yours is still open. Stay silent only when the increment adds
no finding, closes none, and leaves none of yours open.

## A push while a finding is open

While a finding of yours holds approval, every push gets one short update
comment, even when it changes nothing you raised. That comment names the head
you reviewed, links each open thread, and says approval waits on them:

```markdown
## Review Update

Reviewed `d99528e`. This push changes no open finding.

### Still open
- A failed delivery is logged and dropped — [thread](https://github.com/OWNER/REPO/pull/NUMBER#discussion_r123)

Approval waits on this thread.
```

A rebase that leaves the PR's own diff unchanged gets no comment: the last one
still stands. When the only open findings are nits an earlier turn raised, the
push gets the approval pass instead.

## Post-approval check

You approved and a commit landed. Take the first branch that matches; each is a
fact about the increment, not an impression of it:

1. **Substantial new material** — the increment adds a public entry point: an
   endpoint, an exported function or component, a migration, a new module or a
   new dependency. Review it as an update re-review, then run a fresh approval
   pass if it is clean.
2. **A rule is broken** — you can name the rule and the line in the increment
   that breaks it. Post it as a finding with the update format, saying the
   earlier approval no longer reflects the head. Do not dismiss your approval:
   the finding speaks for itself.
3. **Neither** — your judgement stands; post no report. When your latest review
   on the PR still reads `APPROVED`, post nothing. When the repository dismissed
   it on push (`DISMISSED`), re-approve (`SKILL.md` § "Approving") with a one-line body such as
   `Re-approved at a1b2c3d: the push changes only comments.`

A rebase, a formatting pass, a comment edit and a lockfile refresh land in
branch 3. If you cannot name the rule under branch 2, you are in branch 3.

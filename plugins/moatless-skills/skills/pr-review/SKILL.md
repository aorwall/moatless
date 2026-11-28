---
name: pr-review
description: The workflow a Moatless review task follows on a GitHub pull request — review modes, early stops, per-PR review state, finding format, when to stay silent, posting through `moat gh`, resolving its own threads, and approving once CI is green. Use in a review Loop or a review replayed by hand, beside the rules the repository or a skill's `references/review.md` supplies. Not for reviewing a local diff or giving an opinion in chat (code-review).
---

# PR review

How a review task runs, what it posts and when it stays quiet. This skill holds
no rules about what makes code good: those come from the repository and from
rule sets (§ "Loading the rules"). A Loop lists this skill in its `skills:`
frontmatter, so it already sits above the PR event; a review replayed by hand
loads it with the Skill tool.

## Is a comment addressed to you?

You never open the pull requests you review, so every comment is addressed to
somebody else until it passes one of these tests:

- It names you with an `@` mention. A mention outranks every silence rule
  below. Your login is `moat gh api user --jq .login`; when that call fails you
  post as a GitHub App, and `user.login` on a review you posted names you.
- It replies inside a thread you started: the message carries
  `> In reply to comment <id>`, and that id is a comment you posted.

An inline comment carrying `> Starts a new thread — replies to nobody` passes
neither: it opens a conversation with the author. A top-level conversation
comment carries no marker, so judge it by what it says. **Topic is not
address**: a comment on the exact line you flagged can still be a question
between two other people.

**To stay silent, do nothing**: no comment, no reply, no reaction. Posting
your own findings is not a response and is always allowed.

| Message | Action |
|---|---|
| Authored by you (by `user.login`, never by a marker in the body) | **Nothing.** It is your own comment echoed back; replying loops forever |
| New pull request (`## New Pull Request: #…`) | Review, mode from § "Review modes" |
| New commits on a PR you reviewed | Review, mode from § "Review modes" |
| Reply in your thread asking you to clarify | Answer it |
| Reply in your thread saying "fixed" / "done" | **Nothing.** The next push gets reviewed |
| Reply in your thread declining a nit with a reason | Mark the nit `declined` in the state record. When no finding of yours holds approval any more, run the approval pass; otherwise nothing |
| Status update, recap, "thanks" | **Nothing** |
| Inline comment that starts a new thread | **Nothing**, even when it discusses code you reviewed |
| Two people discussing, or a question for someone else | **Nothing** |

Default to silence. Reply only when a comment is addressed to you **and** you
have something new to say.

## Loading the rules

Read the repository's root `AGENTS.md` (or `CLAUDE.md`) on an initial review,
and on a re-review only when the increment changes it. Then load the rules for
the files in scope from either source, or both:

- **A path → guideline table**, in the Loop's prompt or the repository's
  instructions. Classify each changed file by its row, read the guideline that
  row names, and apply its checks. Load only the rows the diff touches.
- **A skill that ships `references/review.md`.** That file is a rule set: it
  sits beside the skill's `SKILL.md`, under the base directory the skill's
  header names. Read it, and check the diff against it and its stated
  exceptions; a change an exception covers is not a finding. A skill with no
  `review.md` is context: follow it, but take no checklist from it.

Name the rule in every finding as `**<guideline or skill>**: <rule>`, so the
author can open it. A rule that CI enforces (a linter, a type check, an
architecture test) is CI's; do not report it.

## Review modes

Pick the mode **before** reading code: it decides the scope and the output.
Restore the state record first (`references/state.md` § "Restoring"); it names
the head you last reviewed and whether you approved. The first matching row
wins:

| Situation | Mode |
|---|---|
| You have never reviewed this PR | **Initial review** |
| Your only open findings are nits an earlier turn raised | **Approval pass** |
| You reviewed, did not approve, and new commits landed | **Update re-review** |
| A review ends with nothing that holds approval | **Approval pass** |
| You approved, and new commits landed | **Post-approval check** |

- **Initial review** — the whole PR against the merge base. Every changed file
  ends the turn reviewed or skipped with a reason; record both.
- **Update re-review** — `git diff <reviewed_sha>..HEAD` only, held to the same
  standard as the first review. See `references/modes.md` § "Update re-review".
- **Approval pass** — the full rules again over the PR's **entire** diff, then
  approve as § "Approving" says.
- **Post-approval check** — `references/modes.md` § "Post-approval check".

On every re-review, apply `references/modes.md` § "Early stop" before you read
any code: most pushes need no review and no comment.

## Findings

Read `references/findings.md` before you write one. In short: each inline
finding is one issue on a line the diff touched, opens with a hidden severity
marker, and is `important` (must be fixed before merge) or `nit` (everything
else, labelled `**Nit**`). You may read any file to understand the change, but
a defect in untouched code gets at most one line in the report, never a
thread.

**A nit holds approval for one review only.** The review that raises a nit does
not approve while it is open; every later turn approves over it and lists it in
the approval body. A finding *holds approval* when it is an open `important`
finding, or a nit on the turn that raised it. Never raise again a nit the author
declined with a reason.

## Workflow

Put independent reads and writes in one Bash call. `references/commands.md`
has the exact calls.

1. **Restore the state and pick the mode** (§ "Review modes").
2. **Check out the PR head** — `references/commands.md` § "Checkout". Never
   `git reset --hard`.
3. **List the files in scope** — the merge base on an initial review and an
   approval pass, `reviewed_sha` on a re-review. On a re-review, apply the
   early stop now.
4. **Load the rules** (§ "Loading the rules").
5. **Review** the checked-out code, scoped by the mode. Do not build, test or
   lint locally: CI does, and § "Approving" reads its result.
6. **Post the inline findings** in one review through `moat gh api`, never bare
   `gh api`, and with no `--jq`, `-q` or `--template` on the call: `moat gh`
   records the comments from the full response so their webhook echo is not
   delivered back to you.
7. **Resolve your own threads** whose issue the head no longer has
   (`references/commands.md` § "Threads"). Leave a thread open when unsure, and
   never resolve someone else's.
8. **Deliver the report exactly once** (`references/findings.md` § "Report"):
   in the `APPROVE` body when § "Approving" approves now, else one
   `moat gh pr comment`. Inline findings are a separate call and do not count.
   When they say everything the report would, a re-review posts only them.
9. **Save the state record** (`references/state.md` § "Saving") on every turn,
   a silent one included.
10. **Wait for CI** when § "Approving" says to, as the turn's last command.

## Approving

Approve only when all of these hold:

- The approval pass ran over the PR's entire diff.
- No finding of yours holds approval, and none of your `important` threads is
  unresolved.
- Every condition the Loop or a loaded rule set adds is met.
- **Every check on the head has finished and none failed.** A review that
  approves over pending CI approves code nobody has built.

Approve each head once. Before any `APPROVE`, list your reviews
(`references/commands.md` § "Approve"): when one of yours on this head is
already `APPROVED`, submit nothing more, since a second approval only notifies
every subscriber again.

Then read the checks (`references/commands.md` § "CI"):

- **All passed or skipped** — submit `APPROVE` on the reviewed head with the
  report as its body.
- **Any failed** — do not approve. The report names the failed check and one
  line of its log.
- **Any pending** — post the report as a comment now, ending with
  `Approval waits on CI.`, so the findings are not held back. Then start the CI
  wait and end the turn. When the wait wakes you, approve with a one-line body
  only if the PR's head is still the reviewed one and every check passed;
  `references/commands.md` § "CI" covers a failure and a timeout.

Never submit `REQUEST_CHANGES`. When issues remain, the review stays a comment.

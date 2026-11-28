# Findings and the report

Read this before you write an inline finding or a report.

## Severity

`important` is a finding that must be fixed before merge: a correctness,
security or data-loss defect, a broken contract, or a rule the loaded rule set
marks as blocking. `nit` is everything else: a name, a placement, a comment, a
test-structure or a convention point.

The first line of every finding's body is a marker the author never sees, and
the state rebuild (`state.md` § "No record") reads severity from it:

```text
<!-- pr-review severity=important -->
<!-- pr-review severity=nit -->
```

A nit also says so where it shows: its visible text opens with `**Nit** · `.
Wherever a report lists a nit, prefix it with `**Nit**` too, so the author can
tell what they may decline.

## Inline comment format

````
<!-- pr-review severity=nit -->
**Nit** · **<rule>**: <one-line statement of what is violated>

<why it matters in this codebase>

```<lang>
// Current
<the problematic code>

// Expected
<the corrected code>
```
````

- One issue per comment; two rules broken on one line share a comment with a
  section each.
- Fence code: unfenced, GitHub joins its lines into a paragraph.
- State a confirmed problem with its trigger and consequence, and give the fix
  as an instruction. For an uncertain one, lead with the possible cause and one
  "may", then the evidence and the next check; do not phrase it as a question.
- Comment only on lines the diff touched, and not on style the rules and the
  repository leave open.

## Report

The full report goes on an initial review and an approval pass. Every line
in it asks the author to do or decide something. Leave out verdicts such as
"this is acceptable", "no concern" or "matches the convention", checklist rows
that pass, and restatements of what the PR does or of CI's result: about a
third of past report bullets were these, and they bury the ones that need
action. Include only the sections with something in them; a Loop or a rule set
may add sections, such as a compliance checklist, where marked.

```markdown
## Code Review

### Progress Summary
<!-- Only on an approval pass that follows an earlier review of yours -->
- Fixed: <issue> — `<file>:<line>`
- **Nit** · Still open: <issue> — [thread](<url>)

<!-- Sections the Loop or a rule set defines go here -->

### Additional Feedback
- <logic, performance, security, structure>
- <one line per defect in untouched code that matters>
```

When everything is clean: `Clean implementation.` and at most one line naming
a pattern the PR got right. Do not hunt for praise.

## Update

The short format, for an update re-review and a post-approval finding. No
checklist, and omit any empty section:

```markdown
## Review Update

### Fixed
- <issue> — `<file>:<line>` *(thread resolved)*

### Still open
- <issue> — [thread](<url>)

### New findings
- **Nit** · <issue> — `<file>:<line>`
```

When everything is fixed and nothing is new, this is an approval pass: post
the full report instead.

## Fixing a mis-rendered comment

Edit it in place rather than posting a "Correction:" comment:
`moat gh api repos/OWNER/REPO/issues/comments/<id> -X PATCH -F body=@<file>`
(`pulls/comments/<id>` for an inline one).

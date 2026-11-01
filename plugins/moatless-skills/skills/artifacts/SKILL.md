---
name: artifacts
description: Build and publish a Moatless Artifact, a self-contained HTML page that outlives the sandbox, shown inline in the conversation or opened from its link, and make it interactive so readers can save data and the task's agent hears about it. Use before writing any page for `moat artifact publish`, when a page should collect input (a form, a vote, a review checklist, feedback on a mockup), or when a message arrives with `<source>artifact</source>`.
---

# Artifacts

An Artifact is one HTML file published from a task with `moat artifact
publish`. It is stored outside the sandbox, so its link keeps working after
the sandbox stops, and any signed-in user can open it, even when the task is
private. Leave it public: `--private` is only for the pages described below.

## Building the page

When they are available, load the `artifact-design` skill before you write
the page, and the `artifact-diagramming` skill before you draw a diagram in it. They own how the
page looks; this skill owns how it is published and how it saves data.

1. **Write one self-contained file in a scratch directory, never the repository.**

   Use `/opt/moatless/workspace/scratchpad/<slug>.html`. Inline the CSS and
   scripts, embed images as data URIs, and load libraries from `cdn.jsdelivr.net`
   and fonts from Google Fonts: the page's CSP blocks every other host. The page
   cannot load a second file you publish, because only the one file is stored.

2. **Keep the page readable without its scripts.**

   The page runs in a sandboxed frame with an opaque origin: `localStorage`,
   `sessionStorage` and cookies throw or are empty. Keep state in memory, or in
   records (see below) when it must survive a reload.

3. **Publish and give the person the printed URL.**

   ```bash
   moat artifact publish scratchpad/report.html --title "Q3 latency report"
   ```

   Publishing again with `--slug <slug>` (derived from the title when
   omitted) adds a version to the same Artifact, and its link always opens the
   latest version. `moat artifact list` shows what the task has published,
   and `moat artifact get <id> -o page.html` downloads a version, such as an
   Artifact a reader forked into this task.

   Publish public by default, including pages about this task's customer or
   private repository: a private page locks out every colleague who cannot
   open the task. Add `--private` only when the page shows what access control
   exists to hide: a secret, a credential, personal data such as users' email
   addresses, or other tasks' content that a reader could not otherwise see.
   On a public page any signed-in user can list and
   save records, and with `--notify` message you; use `--rules` to limit
   what they may write. The
   visibility covers every version, and a publish without `--private` keeps
   it; a person who can change the task can switch it in the thread's
   Artifacts panel or the bar above the page.

Publish finished work meant for other people, such as a report for a team or
the case for a decision, and pages the person asks to view or share. Advice
they will act on right away in the code they are working on needs no page.

## Showing a page in the conversation (`--inline`)

`--inline` shows the page in the thread, above your next reply, in a
borderless frame on the conversation's background. Use it for a chart, a
table, a diagram or a mockup. Run it before you write the final reply. The
reader already sees the page, so do not announce it, paste its URL or restate
it; write only what the page does not say.

- **Leave `html`, `body` and the outermost element without a background, padding or border.**

  The page is part of the reply. A box that needs its own background gets at
  least 16px padding and `var(--radius)` corners.
- **Let the content set the height.**

  The frame grows to fit the page, so `100vh` or `height: 100%` on `html` or
  `body` makes it grow again and again. Give charts fixed pixel heights.
  `--height <px>` (80 to 2000, default 1200) caps the frame, and content past
  it scrolls.
- **Use the theme variables rather than fixed colours.**

  `--background`, `--foreground`, `--muted-foreground`, `--border`,
  `--primary`, `--chart-1` to `--chart-6` and others follow the reader's theme
  and light or dark mode live. `moat artifact publish --help` lists them all.

## Making a page interactive (`--data`, `--notify`, `--rules`)

An interactive page is one readers can answer on: with `--data`, its buttons
and forms save JSON records on the Artifact, which you read with `moat
artifact data`. `--notify` sends you a message for each write, and `--rules`
limits what readers may do. Read
[references/interactive.md](references/interactive.md) before you write a
page with any of them, and when a message arrives with
`<source>artifact</source>`: it holds the page API, rules, the Notify-button
pattern and how to answer a reader.

### When to make a page interactive

Publish with `--data` when the answer you need is easier to give on a page
than in a chat reply:

- **The reader decides item by item.**

  Triage a list of findings, accept or reject review comments, or tick a
  checklist. One record per item, keyed by the item's id, beats a reply that
  lists numbers.
- **The choice is visual.**

  Pick between mockups, colour schemes or chart variants shown side by side.
- **Several people answer.**

  A vote or a team review: anyone who can open the page can save a record. A
  chat question reaches only the person in this thread.
- **The answer comes later.**

  A reviewer opens the link tomorrow. With `--notify` their answer wakes this
  task even when its sandbox has stopped.

Add `--notify` when you will act on each answer as it arrives. Leave it off
when you read the records only when asked, such as a survey you tally at the
end, or when the page saves often, such as a draft saved while the reader
types: every write is a message.
When a reader makes several decisions and you should act on them together,
let them save freely and notify only when they press a button: see
[Notifying on a send](references/interactive.md#notifying-on-a-send-not-on-every-save).

Leave `--data` off when the reader has nothing to answer on the page, such as
a report or a plan they only read.

- **Collect no secrets, tokens or personal data.**

  Every reader of the task can list every record.
- **Keep the outcome somewhere other than the page.**

  Records live only as long as the Artifact. Write the outcome into the
  repository, the PR or the ticket too, and into `moat memory` when later
  tasks need it.

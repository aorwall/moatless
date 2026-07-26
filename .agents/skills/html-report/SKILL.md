---
name: html-report
description: Render an analysis or review as a single self-contained, editorial HTML page — Tailwind + Mermaid via CDN — saved under the workspace reports dir and opened in the browser. Use when the user asks for an HTML or visual report, or another skill needs the report format.
# Forked from https://github.com/mattpocock/skills — maintained locally
---

# HTML Report

Render the deliverable as a single self-contained HTML file. Tailwind and Mermaid both come from CDNs. Mermaid handles graph-shaped diagrams reliably; hand-built divs and inline SVG handle the more editorial visuals. Mix the two — don't lean on Mermaid for everything, it'll start to look generic.

## Mechanics

Write the file to `/opt/moatless/workspace/scratchpad/`, creating the directory if missing. It sits beside the repo checkout rather than inside it, so generated pages never land in a diff.

**Naming depends on whether the page will change.**

| | Name | Because |
|---|---|---|
| One-shot snapshot | `<slug>-<timestamp>.html` | Each run is its own artifact; old ones stay readable |
| Anything you will update | `<slug>.html`, overwritten in place | A fresh name per turn breaks the tab the user already has open |

**Serving it — inside a moatless sandbox** (the normal case), the task UI renders it:

```
<origin>/tasks/<task-id>?file=scratchpad%2F<slug>.html&fileView=preview
```

`file` and `fileView` are search params on the **task route** — not a separate `/file` sub-route — so the link opens the page inside the normal task UI rather than stranding the reader on a bare document.

- **The path is relative to the workspace root** (`WORKSPACE_DIR`, `/opt/moatless/workspace`), not to the repo. That means repo files carry the checkout directory: `moatless/specs/foo.md`, not `specs/foo.md`.
- **URL-encode it.** Slashes become `%2F`. `scratchpad%2Freport.html`, `moatless%2Fspecs%2Ffoo.md`.
- `fileView` is one of `code`, `diff`, `preview`. **`preview` renders both HTML and markdown** (`.md`, `.mdx`) — anything else falls back, so `code` is the only sensible mode for source.

Give the URL once. Don't re-paste it on every update — the tab is already open.

**The same form links any workspace file**, which is usually more useful than pasting its contents:

| Linking | `file=` | `fileView=` |
|---|---|---|
| A generated page | `scratchpad%2F<slug>.html` | `preview` |
| A spec or doc | `moatless%2Fspecs%2F<slug>.md` | `preview` — rendered, not raw |
| Source you're citing | `moatless%2Fbackend%2Fsrc%2F...` | `code` |

Prefer a link over a paste whenever the file is long enough that quoting it would bury the message.

**Update the file with the Write or Edit tool. Never a shell redirect.** The viewer refetches because `Write`/`Edit`/`MultiEdit` tool-call events arrive over SSE and invalidate the task's file queries with `refetchType: 'active'` — exactly the file the user is looking at. A `cat > file` in bash emits no such event, so the page silently goes stale. Don't add a `<meta http-equiv="refresh">` either; the viewer handles it.

This applies to every file the user has open, not just generated pages: edit a spec they are reading with the Edit tool and their tab updates; rewrite it with `sed -i` and it does not.

**Constraints inside the preview.** The iframe renders via `srcDoc` under a sandbox that omits `allow-same-origin`. Scripts and remote subresources load normally — the Tailwind CDN and the Mermaid ESM import both work — but storage APIs (`localStorage`, cookies) do not. Keep the markup semantic enough to read unstyled, in case CDN egress is blocked.

**Outside a sandbox**, fall back to the OS temp dir (`$TMPDIR`, then `/tmp`, or `%TEMP%`) and open it directly: `xdg-open` on Linux, `open` on macOS, `start` on Windows.

## Scaffold

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>{{title}}</title>
    <script src="https://cdn.tailwindcss.com"></script>
    <script type="module">
      import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs";
      mermaid.initialize({ startOnLoad: true, theme: "neutral", securityLevel: "loose" });
    </script>
    <style>
      /* small custom layer for what Tailwind doesn't cover cleanly:
         the caller adds its own classes here — dashed lines,
         hand-drawn-feeling arrow heads, etc. */
    </style>
  </head>
  <body class="bg-stone-50 text-slate-900 font-sans">
    <main class="max-w-5xl mx-auto px-6 py-12 space-y-12">
      <header>...</header>
      <section class="space-y-10">...</section>
    </main>
  </body>
</html>
```

## Choosing a diagram form

Use Mermaid when relationships are graph-shaped (call graphs, dependencies, sequences) — a `flowchart` or `graph` when the point is "X calls Y calls Z, and look at the mess"; sequence diagrams work well for "before: 6 round-trips; after: 1." Wrap it in a Tailwind-styled card so it doesn't feel parachuted in:

```html
<div class="rounded-lg border border-slate-200 bg-white p-4">
  <pre class="mermaid">
    flowchart LR
      A[Caller] --> B[Middleman]
      B --> C[Target]
  </pre>
</div>
```

When Mermaid's layout fights you, hand-build boxes-and-arrows: elements as `<div>`s with borders and labels, arrows as inline SVG `<line>` or `<path>` elements positioned absolutely over a relative container. Reach for this when a visual needs a weight or texture Mermaid won't render.

For before/after comparisons, put the two diagrams side by side in two columns, each ~320px tall so they sit comfortably without scrolling.

## Style baseline

- Lean editorial, not corporate-dashboard. Generous whitespace. Serif optional for headings (`font-serif` works well with stone/slate).
- Colour sparingly: one accent (emerald or indigo) plus red for problems and amber for warnings.
- Use `text-xs uppercase tracking-wider` for labels inside diagrams — they should read as schematic, not as UI.
- Prose is sparse and plain; the diagrams carry the weight. If a diagram needs a paragraph to be understood, redraw the diagram.
- The only scripts are the Tailwind CDN and the Mermaid ESM import. The report is otherwise static — no app code, no interactivity beyond Mermaid's own rendering.
- **Read-only.** No forms, no buttons, no answering by clicking. The user acts in chat; the page holds state they would otherwise have to.

## Skills that build on this

Everything above is the shared base: where the file goes, how it's served, how it's updated, and what it looks like. Skills that produce a *particular kind* of page own their own layout on top of it, and none of them restate the mechanics.

| Skill | Its page | Layer |
|---|---|---|
| `/writing-specs` | A review page at approval — nothing during the interview | [HTML-REPORT.md](../writing-specs/HTML-REPORT.md) |
| `/explaining-code` | An explanation of how existing code works | [HTML-REPORT.md](../explaining-code/HTML-REPORT.md) |

If you are producing one of those, read its layer — it fixes the section order and the caps, which is most of what makes the page useful. For anything else, this file is enough on its own.

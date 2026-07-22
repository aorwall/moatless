---
name: html-report
description: Render an analysis or review as a single self-contained, editorial HTML page — Tailwind + Mermaid via CDN — written to the OS temp dir and opened in the browser. Use when the user asks for an HTML or visual report, or another skill needs the report format.
# Forked from https://github.com/mattpocock/skills — maintained locally
---

# HTML Report

Render the deliverable as a single self-contained HTML file. Tailwind and Mermaid both come from CDNs. Mermaid handles graph-shaped diagrams reliably; hand-built divs and inline SVG handle the more editorial visuals. Mix the two — don't lean on Mermaid for everything, it'll start to look generic.

## Mechanics

Write the file to the OS temp directory so nothing lands in the repo. Resolve the temp dir from `$TMPDIR`, falling back to `/tmp` (or `%TEMP%` on Windows), and write to `<tmpdir>/<slug>-<timestamp>.html` so each run gets a fresh file. Open it for the user — `xdg-open <path>` on Linux, `open <path>` on macOS, `start <path>` on Windows — and tell them the absolute path.

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

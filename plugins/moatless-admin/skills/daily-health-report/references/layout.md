# Layout

`scripts/layout.mjs` is an ESM module with no dependencies. Every function returns an HTML string, so a section it has no component for is written as an ordinary template literal in the same array. The module owns the document shell, the CSS variables and the chart geometry; it owns nothing about what the page says.

```js
import { page, section, tiles, card, columns, hbars, table, swatch, esc, num, SERIES } from "./layout.mjs";
```

## The shell

**`page({ title, env, heading, lede, sources, body, footer })`** returns the whole document: Tailwind CDN, the Mermaid ESM import, the CSS block, and a `max-w-5xl` main column. `body` is the concatenated sections in render order — `page` imposes no order of its own, so the order in SKILL.md is yours to hold.

`env`, `lede`, `sources` and `footer` are inserted as raw HTML so they can carry `<code>`, `<strong>` and entities. `heading` and every component's `heading` go through `esc`.

**`section({ heading, body, space = 5 })`** emits a horizontal rule, one `h2`, then `body`, inside `space-y-<space>`. There is no kicker slot, deliberately — the section's name is its heading. The default spacing suits every section on this page; raise `space` only for a section whose children are multi-paragraph blocks rather than cards.

## Components

**`tiles(items)`** — the stat row, `items` of `{ label, value, sub, tone }`. The grid is two columns on mobile and four above it, so pass four or eight. `tone` is a CSS variable name: `critical`, `serious`, `warning`, `good`. `value` and `sub` are raw HTML; format numbers with `num()` first.

**`card({ heading, right, note, body })`** — a bordered panel. `right` sits opposite the heading for a total or a unit ("2,066 events · UTC"). `note` is one muted line under it, for the sentence that says how to read the chart. Omit `heading` for a bare panel.

**`columns({ labels, series, peaks = 1, tips = labels })`** — a column chart, stacked when `series` has more than one entry. `tips` replaces `labels` inside the tooltip, where there is room for a fuller form: pass `"08"` for the axis and `"08:00"` for the tooltip. `series` is `[{ name, values }]` with `values` parallel to `labels`. Heights are a percentage of the tallest column, so pass raw counts and never pre-scale. `peaks` direct-labels that many tallest columns — plus any column tied with the last of them, so a flat series can print more labels than you asked for. Every column carries the full breakdown in its `title` tooltip.

Two or more series need a legend or a table beside the chart — colour alone does not identify them. `swatch(i)` returns the square for series `i`, for a legend line or a table's first cell.

**`hbars(rows)`** — a ranked list, `rows` of `{ label, value, text, color }`. Width is relative to the largest row. `text` overrides the printed value when the unit is not a plain count ("$212.44", "2.69 M lines"). `color` is a series variable, defaulting to `s1`. The label column is a fixed 190px, so keep labels short.

**`table({ cols, rows, foot })`** — `cols` entries are strings, or `{ h, n: true }` to right-align the column as a number. A row is an array of raw-HTML cells. `foot` renders bold, for a total line.

**`num(v, d = 0)`** formats with thousands separators and at most `d` decimals. **`esc(s)`** escapes for a raw insertion point.

## Colours

`SERIES` is `["s1","s2","s3","s4"]` — blue, orange, green, amber — assigned in fixed order and never cycled. A fifth category folds into an "Other" row rather than getting a new hue. The status variables `--good`, `--warning`, `--serious` and `--critical` are reserved for state and never used as a series colour. Every variable has a dark-mode value, so a custom component that uses `var(--…)` follows the reader's theme for free and one that hard-codes `#fff` does not.

## The fix card

`Three things to fix` has no component — it is the plain-HTML block in `scripts/example-fixes.html`, built from the same classes the components use: `.card p-5` for the panel, `.kicker` for the Problem and Fix labels, `.ink2` for the problem text, `.rule` for the divider under the header, `.tnum` for the number. Severity is a left border taking its colour from a status variable, `style="border-left:3px solid var(--critical)"`.

Nothing in it may use a Tailwind palette utility. `bg-slate-900 text-white` renders a black badge on a `#1a1a19` card once the reader's system is in dark mode, and the page has no `dark:` variants to rescue it — the CSS variables are what follow the theme. SKILL.md holds the slot-by-slot rules and the full markup.

## Writing a custom component

A fix sometimes needs one visual nothing here draws — a timeline, a before/after pair, a Mermaid graph. Write it inline:

```js
const timeline = card({ heading: "Provisioning attempt", body: `
          <pre class="mermaid">
            sequenceDiagram
              participant M as message wake
              participant P as auto_provision
              M->>P: 12:52:31 preflight failed
              P->>P: 12:58:39 Deployment built anyway
          </pre>` });
```

What a custom block must match to sit beside the generated ones:

- **Wrap it in `card(...)`** rather than hand-rolling a panel, so the border, radius and surface match.
- **Take colour from the CSS variables**, never a literal hex.
- **Carry no client script and no form.** The page renders in the task viewer's sandboxed iframe, which omits `allow-same-origin` — storage APIs fail there. Tailwind and the Mermaid import are the only scripts. The page is read-only; the reader acts in chat.
- **Keep it readable unstyled**, in case CDN egress is blocked: real headings, real tables, no layout that depends on a class to make sense.
- **Escape anything from the data** with `esc()`. A repo name or a log line reaches the page verbatim otherwise, and one `<` truncates the document.

## Mermaid or hand-built

Mermaid for graph-shaped relationships — a call path, a sequence, a dependency. Divs and the components above for anything quantitative, because a chart built from percentage heights renders identically everywhere and a Mermaid chart of numbers does not. If a diagram needs a paragraph before it makes sense, redraw the diagram.

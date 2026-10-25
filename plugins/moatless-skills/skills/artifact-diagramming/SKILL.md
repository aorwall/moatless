---
name: artifact-diagramming
description: Draw a diagram that shows a real mechanism in a Moatless Artifact page — what to draw, inline SVG that follows the reader's theme, and Mermaid for graph-shaped layouts. Use when a page needs an architecture, flow, sequence, state or before/after diagram, or when another skill needs the diagram rules.
---

# Artifact Diagramming

Draw as the engineer who has to live with the decision. A diagram earns its
place when it shows a cold reader a mechanism they would otherwise assemble
from prose: where data flows, which components talk, what changes between two
options, what state a request moves through. When a sentence says it faster,
write the sentence instead.

## What to draw

**Show the mechanism, not its name.** A box labelled "cache" says less than the
prose. The path a request takes through it, the two stores it sits between,
and the arrow that disappears without it say what words cannot. Draw the parts
the argument depends on (the boundary crossed, the hop added, the data moved)
and leave out the rest.

**Comparing options: draw the difference.** Put the two architectures side by
side, or before and after, so the reader can point at the edge each option adds
or removes. A separate box per option, connected to nothing, is a restated
list, not a comparison.

**Match complexity to the stakes.** A one-hop question is three boxes. A
migration that reroutes writes through a queue needs the queue, the writer,
the reader and the ordering arrow. Draw what the decision turns on, not an
inventory of the system.

**Label the arrows.** `writes`, `invalidates`, `polls every 30s` are
information; an unlabelled arrow is not. Add a legend only when one encoding
(dashed, coloured, doubled) repeats; otherwise put the meaning on the mark.

Done when every box and arrow in the drawing is one the caption's claim needs.

## Choosing the form

| Shape of the content | Form |
|---|---|
| A graph whose layout an engine should solve: call graphs, dependencies, sequences, state machines | Mermaid |
| A drawing whose position carries meaning: layers, boundaries, a before/after pair, one highlighted hop | Inline SVG |

## Inline SVG

Hand-write `<svg>` with `rect`, `circle`, `line`, `polyline`, `path` and
`text`. No library, no runtime, no external image.

- **Size by `viewBox`.** Set `viewBox="0 0 W H"` for the content and let CSS
  scale it (`width: 100%; height: auto`). Wide flows read left to right;
  stacks read top to bottom.
- **Draw a narrow version of a wide figure.** Scaled to 360px, a 640px drawing
  sets its labels near 6px, and a scroll container hides half the point. Draw a
  second `<svg>` that turns the flow top to bottom, and switch the two with
  `@media (max-width: 560px)`. Two narrow figures side by side, such as a
  before and an after, can instead stack to one column.
- **Theme with `currentColor` and the page's palette tokens.** Strokes, text and
  arrowheads in `currentColor` follow the page's ink in both themes. Set token
  colours through `style` or a class (`style="fill: var(--panel)"`), not a
  presentation attribute such as `fill="var(--panel)"`, which not every browser
  resolves. Give one role colour only to the element that carries the point.
- **Show the mechanism as a shape.** A race is a shaded window, a boundary is a
  dashed frame, a removed hop is a struck edge. Put the figure in a panel with
  its own fill when the ground around it is busy.
- **Give every shape an explicit `fill`.** An SVG shape defaults to black fill,
  which hides text on a dark theme. Use `fill="none"` on lines and paths.
- **Arrowheads are markers or polygons.** A `<defs><marker>` referenced by
  `marker-end="url(#arrow)"`, with the marker's path filled `currentColor`.
  The marker takes `currentColor` from its own `<svg>`, not from the line that
  uses it, so an arrowhead in a role colour needs its own marker.
- **Keep text legible.** 11–13px at the drawn scale, `text-anchor` for
  alignment, labels of one to three words. Sentences go in the caption.
- **Align to a grid.** Shared baselines and even gaps make a hand drawing read
  as deliberate.
- **One figure, one claim.** Wrap the `<svg>` in `<figure>` with a
  `<figcaption>` that states what the picture shows. Give the `<svg>`
  `role="img"` and an `aria-label` with the same claim.
- **Keep ids unique on the page.** Two figures that both define `#arrow` share
  the first one's marker. Prefix ids per figure.

## Mermaid

Moatless does not render Mermaid itself. Load it pinned, and theme it from the
page's tokens. `getComputedStyle` returns a `light-dark()` token unresolved, so
read colours from elements that use them; the example reads the host's resolved
tokens instead:

```html
<figure>
  <pre class="mermaid">
flowchart LR
  A[Caller] -- "GET /user" --> B[Cache]
  B -- "miss: SELECT" --> C[(Postgres)]
  </pre>
  <figcaption>A cache miss costs one extra hop to Postgres.</figcaption>
</figure>
<script type="module">
  import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs";
  const css = getComputedStyle(document.documentElement);
  const v = (name) => css.getPropertyValue(name).trim();
  mermaid.initialize({
    startOnLoad: false,
    theme: "base",
    themeVariables: {
      fontFamily: v("--font-sans"),
      background: v("--background"),
      primaryColor: v("--card"),
      primaryTextColor: v("--foreground"),
      primaryBorderColor: v("--border"),
      lineColor: v("--muted-foreground"),
      textColor: v("--foreground"),
    },
  });
  await mermaid.run();
</script>
```

- **Mermaid bakes the colours in at render time.** A reader who switches theme
  after load sees the old colours until reload. Use inline SVG for a figure
  that must follow a live switch.
- **Put the `<pre class="mermaid">` text flush left.** Indented lines inside the
  block can break the parse.
- **Quote edge labels with spaces or punctuation** (`-- "miss: SELECT" -->`).

Done when each figure has a caption with its claim, reads in light and dark,
and has labels of at least 10px at 360px wide.

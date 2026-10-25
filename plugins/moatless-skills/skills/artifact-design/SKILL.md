---
name: artifact-design
description: Design guidance for a Moatless Artifact page — treatment, palette and type plan, layout, copy and the one check before publishing. Use before writing any HTML page for `moat artifact publish`, or when another skill needs the design pass for a page.
---

# Artifact Design

Work the way the design lead at a small, versatile studio would: give each page
a visual identity at the level of treatment the request calls for. Make
deliberate choices about palette, typography and layout that are specific to
this subject, and avoid templated designs.

The `artifacts` skill owns the publish mechanics: the scratch path, `--inline`,
`--data` and records. This skill owns how the page looks and reads.

## Page contract

**Theme.** Moatless injects a stylesheet into every page that sets `color-scheme`
on `:root`: from the OS setting by default, and from the reader's app theme,
live, when the page is framed in the conversation. It also sets the host tokens
`moat artifact publish --help` lists (`--background`, `--foreground`,
`--radius`, `--chart-1` … `--chart-6` and others) and `html { font-size: 14px }`,
so `1rem` is 14px. Define the page's own palette with `light-dark()`, which
follows that `color-scheme`:

```css
:root {
  --paper: light-dark(#f4f6f7, #0e1418);
  --ink: light-dark(#142029, #e2e9ed);
  --signal: light-dark(#b25e09, #f0a33c);
}
```

- **Do not use `@media (prefers-color-scheme)` or `[data-theme]` blocks.** They follow the OS, and the reader's app theme can differ from it; only `color-scheme` follows both.
- **Define every colour as a token and use only tokens in component rules.** A literal colour reads in one theme only.
- A page that commits to one look (an arcade screen, a print proof) may use plain values. Then set `color-scheme` on `:root` to match, and set the background and every colour explicitly.

**Ground.** The host stylesheet paints `html` with `var(--background)`, the
conversation's own colour. A page published with `--inline` leaves `body` and
its outermost element without a background, so it reads as part of the reply.
Panels, figures and bands inside take their fills from the page's palette. A
page opened only from its link may paint `body` with its own background token.
Write host tokens with a fallback (`var(--radius, 10px)`) so a local preview
without the host stylesheet still renders.

**Frame.** The page runs in a sandboxed frame without `allow-same-origin`,
`allow-modals` or `allow-downloads`:

- `alert()`, `confirm()` and `prompt()` do nothing. Build any confirmation step into the page.
- A download link or a script-driven save does nothing. Show the content as selectable text.
- `navigator.clipboard.writeText` can reject. Call it in the click handler, catch the rejection, and select the text instead.
- `localStorage`, `sessionStorage` and cookies throw `SecurityError`. Keep state in memory, or in the `artifacts` skill's records when it must survive a reload.
- Links open in a new tab through the host. Use real `<a href>` links.

**Libraries and fonts.** The page's CSP loads scripts and styles from
`cdn.jsdelivr.net` only, fonts from Google Fonts or jsdelivr, and images only as
`data:` or `blob:` URLs; a `fetch` reaches the Moatless backend and nothing else,
and an `<iframe>` loads nothing. Pin every
script URL to an exact version (`cdn.jsdelivr.net/npm/chart.js@4.4.1/dist/chart.umd.min.js`),
never a bare name, a range or `@latest`, and place it before the inline script
that uses it. Load fonts from Google Fonts and give every face a real fallback
stack. Inline the page's own CSS, JS and images (as data URIs): only the one
file is stored.

**Responsive.** The frame is as wide as the reply column: 728px on desktop by
default and about 360px on phones. The page must work at 360px, and its body
must never scroll sideways. An inline page fills the frame width with no side
gutter on its outermost element, and caps running text near 65 characters. A
page opened from its link keeps a 16px side gutter and its own `max-width`.

**Title.** `--title` is the page's name in the task's artifact list: a short noun
phrase specific to the subject, typically two to four words, or the one
question the page answers. Use the person's own name for the thing when they
have one. Do not add an explanation after a dash or colon. Keep it stable across
republishes.

## Read the request first

Decide the treatment. A report gets the same craft as a
landing page; only the treatment differs.

Many requests call for a utilitarian treatment: a report, a plan, a review, a
memo, a demo. Make it polished, with real typographic hierarchy, considered
spacing and a proper palette, but avoid over-designing. Most pages do not need a
gigantic hero. Keep flourishes tasteful and limited.

Some requests call for an editorial treatment: a landing page, a game, an app
or tool people keep or share. Follow [Editorial pages](#editorial-pages) after
the fundamentals.

If unsure: a well-composed page is always acceptable; an over-designed visual
identity sometimes is not.

## Fundamentals for every page

**Respect what already exists.** Look for an existing design system first: an
`AGENTS.md`, a tokens or theme file, existing component styles. When one exists,
apply it; everything below fills gaps. Precedence is the person's own words,
then the project's system, then your choices.

**Ground it in the subject.** Define one concrete subject, its reader, and the
page's single job. Distinctive choices come from the subject's own world: its
materials, instruments and vernacular. Include at least one detail only this
subject has (its real units and scales, its document conventions, its terms of
art) as content rather than ornament. Use real content, never lorem ipsum.

**Pair typefaces.** Typography decides how the page reads even when the page is
not about typography. Pair a display face with a body face, and add a utility
face for data or code when the page has any. Keep running text near 65
characters wide. Set a type scale and keep to it. Give headings `text-wrap:
balance`, give body text comfortable line height, and give uppercase labels a
little letter-spacing.

**Choose neutrals deliberately.** A pure mid-grey looks unconsidered; a grey with a
slight hue bias toward the page's accent looks intentional. The host's
`--background` and `--muted` greys are the conversation's, not the page's:
build the page's own neutrals.

**Design both themes.** Give the dark values the same care as the light ones. Do
not simply invert them; keep contrast legible and keep the accent working on
both grounds.

**Use layout for spacing.** Lay out sibling groups with flex or grid and `gap`
instead of per-element margins, which collapse or double without warning. Let
rows wrap or stack to one column at phone width. Give any flex or grid child
that holds running text, code or a table `min-width: 0`. Give images and
`aspect-ratio` boxes `max-width: 100%`, and give nothing a `min-width` wider than
the screen. Only wide tables and long code blocks may exceed it, each in its own
`overflow-x: auto` container. A short quote whose point would scroll out of
view, such as a header or one SQL statement, breaks onto more lines instead. A
diagram gets a narrow version (see `artifact-diagramming`). Use `font-variant-numeric: tabular-nums` wherever
digits line up.

**Make repeated elements consistent.** For cards in a row, label/value pairs
down a list or badges on sibling items, use the same edges, baselines and inner
padding on each. Let content set a container's height and pick a column count
the items fill, so nothing stretches over empty space or sits alone in a row.
Clipped text is a bug.

**Use card styling selectively.** Border, fill, radius and shadow each mark an
element as a separate object. Apply them by role, to set off the one element
that needs it; the same radius and shadow on every block flattens the
hierarchy. Open with a big number only when that figure is the point of the
page.

**Draw charts to scale.** Load the `dataviz` skill, when it is available, before
the first line of chart code. Place marks, ticks and labels with one scale, and
label only values the chart reaches. Colour chart text from the palette tokens.
In SVG, leave room in the `viewBox` for the outermost labels and give every
shape an explicit fill.

**Draw diagrams that show the mechanism.** Load the `artifact-diagramming` skill,
when it is available, before you draw one.

**Make the page complete at rest.** Everything meant to be read is visible once
the page has loaded, with no scrolling to trigger it. A section may animate in,
but from a visible resting state, never from `opacity: 0`. Size a hero to its
content, not to the viewport: `100vh` also makes an inline frame grow without
end. A tool opens in a realistic working state: real data where it exists,
otherwise example rows clearly marked as examples.

**Avoid the generated look.** Generated design clusters around a few looks: warm
cream with a serif display and terracotta accent; near-black with one
acid-green or vermilion pop; broadsheet hairline rules with dense columns; a
purple-to-blue gradient hero on white; Inter or Space Grotesk as the safe face;
emoji as section markers; everything centred; the same rounded corners
everywhere; an accent rail on rounded cards. When the person names a visual
direction, follow it exactly, including one of these. Otherwise do not spend
the freedom on one of these defaults.

**Build cleanly.** Watch for overlapping elements, cascade collisions and silent
font fallbacks. Close every non-void element, double-quote attributes, give
keyboard focus a visible state, and respect `prefers-reduced-motion`. Give
every form control a stable `id`. Watch selector specificity: a type selector
and a class selector that both set spacing undo each other. Use Canvas, not
long hand-written SVG paths, for generative graphics.

**Write the copy.** Words are design material. Write from the reader's side:
name things by what people recognise, not by how the system is built. Use the
active voice; a control says what it does ("Send"), and its result says what
happened ("Sent"). An error says what went wrong and how to fix it. Write
plainly, in short sentences. Skip em-dash asides, "not X, but Y" framing,
colon-then-reveal sentences, scare quotes and stock phrases.

**Structure is information.** Numbering, eyebrows, dividers and labels encode
something true about the content. Number sections only when the order is
information the reader needs.

**UI pages (dashboards, tools).** They are scanned and operated, so the craft
shifts from typography to information design. Put the summary before the
detail. Show state in form as well as in numbers: a chip, a severity stripe.
Semantic colour (good, warning, critical) is separate from the accent. Give
sparklines the same care as type: an area fill, a faint grid, an emphasised
endpoint. Interactive elements look interactive.

## Process

1. **Start from what the reader should do on the page.** When it takes input,
   follow the `artifacts` skill's records section and design around it.
2. **Write the design plan into the file** as the first `<style>` rule, not into
   the reply:
   - **Layout:** the layout concept as a one-line comment.
   - **Colour:** 4–6 named colour tokens, each a `light-dark()` pair.
   - **Type:** font tokens for 2+ roles: a characterful display face used with
     restraint, a complementary body face, and a utility face for data if needed.

   Done when every later rule takes its colours and fonts from these tokens.
3. **Build the page from the tokens.** Say at most one plain sentence about the
   design direction in the reply, with no hex values or font names.
4. **Check once, then publish.** When the sandbox has Playwright, take one set of
   screenshots of the local file at 728px and 360px wide, in both `colorScheme`
   settings, and read `scrollWidth` against the width in the same run. Make one
   pass of fixes for what that set shows, and keep each fix to the defect it
   targets. A local file has no host
   stylesheet, so add `<style>:root{color-scheme:light dark}</style>` to the
   copy you screenshot. For a page that charts real numbers, spend the look on
   the chart. For a page whose point is logic (dates, money, scoring), you may
   also run one pure function on one sample input. Then publish with the
   `artifacts` skill.

   Do not loop: no second screenshot, no DOM probes. Further polish is for the
   person to ask for. When they report something visibly broken, fix it, take
   at most one more look, and republish once.

Done when the page is published and the person has the link or the inline page.

## Editorial pages

A templated page fails here. Make opinionated decisions, and take one real
aesthetic risk where it serves the work.

Review the design plan against the subject before building. Revise any part
that reads like the default you would produce for any similar page, without
narrating the revision. Write the code only once the plan is specific to this
subject, and follow it exactly.

- **The hero states the thesis.** Open with the most characteristic thing in the subject's world: a headline, an image, a live demo, an interactive moment.
- **Typography sets the personality.** Pair display and body faces you would not use on any other project, with a clear scale and deliberate weights, widths and spacing. Make the type treatment a memorable part of the design.
- **Use motion deliberately.** One orchestrated moment, such as a page-load sequence, usually does more than scattered effects.
- **Match complexity to the vision.** A maximalist direction needs elaborate execution; a minimal one needs precision in spacing, type and detail.
- **Put the boldness in one place and keep everything around it quiet.** If the accent clashes with the ground, shift it to an analogous hue or desaturate it rather than replacing it.

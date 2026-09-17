// Page shell, cards and charts for the daily health report. Import from a
// build script; every function returns an HTML string, so a section this file
// has no component for is written inline as ordinary HTML beside the ones it does.
//
// Charts are divs with percentage heights: no chart library, no client script,
// so the page renders inside the task viewer's sandboxed iframe.

/** Series colors, in assignment order. A fifth series folds into "Other". */
export const SERIES = ["s1", "s2", "s3", "s4"];

export const esc = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** Thousands separators, and at most `d` decimals. */
export const num = (v, d = 0) =>
  Number(v).toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });

const tone = (t) => (t ? ` style="color:var(--${t})"` : "");

// ---------------------------------------------------------------- page shell

/**
 * The whole document. `body` is the concatenated sections, in the order they
 * render — this function imposes no section order of its own.
 */
export function page({ title, env, heading, lede, sources, body, footer }) {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${esc(title)}</title>
    <script src="https://cdn.tailwindcss.com"></script>
    <script type="module">
      import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs";
      mermaid.initialize({ startOnLoad: true, theme: "neutral", securityLevel: "loose" });
    </script>
    <style>${CSS}</style>
  </head>

  <body>
    <main class="max-w-5xl mx-auto px-6 py-12 space-y-12">

      <header class="space-y-3">
        <p class="kicker">${env}</p>
        <h1 class="text-4xl font-semibold tracking-tight">${esc(heading)}</h1>
        <p class="text-lg ink2 max-w-3xl">${lede}</p>
        <p class="text-sm muted">${sources}</p>
      </header>
${body}

      <footer class="rule pt-6 pb-4 text-xs muted">${footer}</footer>
    </main>
  </body>
</html>
`;
}

/** A top-level section: a rule, one heading, then its content. No kicker. */
export function section({ heading, body, space = 5 }) {
  return `
      <section class="space-y-${space}">
        <div class="rule pt-8"></div>
        <h2 class="text-2xl font-semibold tracking-tight">${esc(heading)}</h2>
${body}
      </section>`;
}

/** The stat row under the header. Eight tiles fill the 4-column grid evenly. */
export function tiles(items) {
  const cells = items
    .map(
      (t) => `          <div class="card p-4">
            <p class="kicker">${esc(t.label)}</p>
            <p class="text-3xl font-semibold mt-1"${tone(t.tone)}>${t.value}</p>
            <p class="text-xs ink2 mt-1">${t.sub ?? ""}</p>
          </div>`,
    )
    .join("\n");
  return `
      <section>
        <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
${cells}
        </div>
      </section>`;
}

/** `heading` on the left of the card's top line, `right` opposite it. */
export function card({ heading, right, note, body }) {
  const head = heading
    ? right
      ? `          <div class="flex items-baseline justify-between mb-1">
            <h3 class="font-semibold">${esc(heading)}</h3>
            <span class="text-xs muted">${right}</span>
          </div>\n`
      : `          <h3 class="font-semibold mb-1">${esc(heading)}</h3>\n`
    : "";
  return `        <div class="card p-5">
${head}${note ? `          <p class="text-xs muted mb-3">${note}</p>\n` : ""}${body}
        </div>`;
}

// -------------------------------------------------------------------- charts

/**
 * A column chart, stacked when given more than one series. Heights are a
 * percentage of the tallest column, so a series' absolute scale never leaves
 * this function — pass raw counts.
 *
 * `peaks` labels that many tallest columns, plus anything tied with the last of
 * them; the rest are read from the tooltip.
 *
 * `tips` replaces `labels` inside the tooltip, where there is room for a fuller
 * form — "08:00" against an axis that only fits "08".
 */
export function columns({ labels, series, peaks = 1, tips = labels }) {
  const totals = labels.map((_, i) => series.reduce((a, s) => a + (s.values[i] || 0), 0));
  const max = Math.max(...totals, 1);
  const cut = [...totals].sort((a, b) => b - a)[Math.max(0, peaks - 1)] ?? Infinity;

  const cols = labels.map((label, i) => {
    const tip = series.map((s) => `${s.name} ${num(s.values[i] || 0)}`).join(", ");
    const segs = series
      .map((s, k) =>
        (s.values[i] || 0) > 0
          ? `<div class="seg ${SERIES[k]}" style="height:${((s.values[i] / max) * 100).toFixed(2)}%"></div>`
          : "",
      )
      .join("");
    const blab = totals[i] >= cut && totals[i] > 0 ? `<span class="blab">${num(totals[i])}</span>` : "";
    const box = series.length > 1 ? "stack" : "plot";
    return `<div class="col" title="${esc(tips[i])} &middot; ${esc(tip)}">${blab}<div class="${box}">${segs}</div><div class="xt">${esc(label)}</div></div>`;
  });
  return `          <div class="chart axis">\n${cols.join("\n")}\n          </div>`;
}

/** Horizontal bars for a ranked list. Width is relative to the largest row. */
export function hbars(rows) {
  const max = Math.max(...rows.map((r) => r.value), 1);
  return `          <div class="space-y-1">
${rows
  .map(
    (r) => `            <div class="hbar-row"><span class="text-sm">${r.label}</span>
              <div class="hbar-track"><div class="hbar" style="width:${((r.value / max) * 100).toFixed(1)}%;background:var(--${r.color ?? "s1"})"></div></div>
              <span class="text-sm tnum">${r.text ?? num(r.value)}</span></div>`,
  )
  .join("\n")}
          </div>`;
}

/** A colored square naming a chart series in a table cell or a legend. */
export const swatch = (i) => `<span class="swatch" style="background:var(--${SERIES[i]})"></span>`;

/**
 * `cols` entries are strings, or `{h, n}` where `n` right-aligns the column as
 * a number. A row is an array of cells; `foot` renders bold, for a total line.
 */
export function table({ cols, rows, foot }) {
  const cls = cols.map((c) => (typeof c === "object" && c.n ? ' class="n"' : ""));
  const tr = (cells, bold) =>
    `              <tr>${cells
      .map((c, i) => `<td${bold ? ` class="${cls[i] ? "n " : ""}font-semibold"` : cls[i]}>${c}</td>`)
      .join("")}</tr>`;
  return `          <table>
            <thead><tr>${cols
      .map((c) => `<th${typeof c === "object" && c.n ? ' class="n"' : ""}>${esc(typeof c === "object" ? c.h : c)}</th>`)
      .join("")}</tr></thead>
            <tbody>
${rows.map((r) => tr(r)).join("\n")}${foot ? "\n" + tr(foot, true) : ""}
            </tbody>
          </table>`;
}

// ----------------------------------------------------------------------- CSS

const CSS = `
      :root {
        color-scheme: light;
        --page:      #f9f9f7;
        --surface:   #fcfcfb;
        --ink:       #0b0b0b;
        --ink-2:     #52514e;
        --muted:     #898781;
        --grid:      #e1e0d9;
        --baseline:  #c3c2b7;
        --ring:      rgba(11,11,11,0.10);
        --s1:        #2a78d6;
        --s2:        #eb6834;
        --s3:        #1baf7a;
        --s4:        #eda100;
        --good:      #0ca30c;
        --warning:   #fab219;
        --serious:   #ec835a;
        --critical:  #d03b3b;
      }
      @media (prefers-color-scheme: dark) {
        :root:where(:not([data-theme="light"])) {
          color-scheme: dark;
          --page:     #0d0d0d;
          --surface:  #1a1a19;
          --ink:      #ffffff;
          --ink-2:    #c3c2b7;
          --muted:    #898781;
          --grid:     #2c2c2a;
          --baseline: #383835;
          --ring:     rgba(255,255,255,0.10);
          --s1:       #3987e5;
          --s2:       #d95926;
          --s3:       #199e70;
          --s4:       #c98500;
        }
      }
      body { background: var(--page); color: var(--ink);
             font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
      .card { background: var(--surface); border: 1px solid var(--ring); border-radius: 10px; }
      .ink2 { color: var(--ink-2); }
      .muted { color: var(--muted); }
      .rule { border-top: 1px solid var(--grid); }
      .tnum { font-variant-numeric: tabular-nums; }

      .chart { display: flex; gap: 3px; align-items: flex-end; padding-top: 20px; }
      .col { flex: 1 1 0; position: relative; min-width: 0; }
      .plot { height: 176px; display: flex; flex-direction: column-reverse;
              justify-content: flex-start; }
      .stack { height: 176px; display: flex; flex-direction: column-reverse;
               justify-content: flex-start; gap: 2px; }
      .bar, .seg { width: 100%; border-radius: 4px 4px 0 0; }
      .seg:not(:last-child) { border-radius: 0; }
      .s1 { background: var(--s1); } .s2 { background: var(--s2); }
      .s3 { background: var(--s3); } .s4 { background: var(--s4); }
      .xt { margin-top: 6px; font-size: 10px; text-align: center; color: var(--muted);
            font-variant-numeric: tabular-nums; }
      .blab { position: absolute; top: -18px; left: -6px; right: -6px; text-align: center;
              font-size: 11px; font-weight: 600; color: var(--ink);
              font-variant-numeric: tabular-nums; }
      .axis { border-bottom: 1px solid var(--baseline); }

      .hbar-row { display: grid; grid-template-columns: 190px 1fr 92px; gap: 10px;
                  align-items: center; padding: 3px 0; }
      .hbar-track { height: 14px; }
      .hbar { height: 14px; border-radius: 0 4px 4px 0; }

      .swatch { display: inline-block; width: 11px; height: 11px; border-radius: 3px;
                vertical-align: -1px; margin-right: 6px; }
      table { width: 100%; border-collapse: collapse; }
      th { text-align: left; font-size: 11px; text-transform: uppercase;
           letter-spacing: .06em; color: var(--muted); font-weight: 600;
           padding: 6px 8px; border-bottom: 1px solid var(--grid); }
      td { padding: 6px 8px; border-bottom: 1px solid var(--grid); font-size: 13px;
           vertical-align: top; }
      td.n { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
      code { font-size: 12px; background: var(--page); padding: 1px 4px; border-radius: 3px;
             border: 1px solid var(--ring); }
      .kicker { font-size: 11px; text-transform: uppercase; letter-spacing: .08em;
                color: var(--muted); font-weight: 600; }
      .mermaid { background: transparent; }
`;

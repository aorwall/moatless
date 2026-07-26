# The explanation page

**Read [`/html-report`](../html-report/SKILL.md) first** — it owns where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. This file adds only the layout for an explanation.

Naming: `scratchpad/<slug>-explained.html`. Use the **stable** rule — these get revisited and extended as follow-up questions land, and a timestamped name would strand the tab.

```
<origin>/tasks/<task-id>?file=scratchpad%2F<slug>-explained.html&fileView=preview
```

**This page is the artifact, not a projection of one.** There is no markdown behind it, so it carries everything — write it directly and completely rather than as a rendering of something else. `SKILL.md` decides whether it gets built; by the time you are here, it does.

## Layout

Same five slots as the chat answer, in the same order — the point of a fixed order is that it holds across mediums.

**1. The answer, as the header.** One or two sentences at full size, immediately under the title. This is the mechanism, and it should be legible from across the room. Everything below is support.

**2. Vocabulary, as a compact table.** Skip it entirely after a calibration that said "I know this area". Four rows maximum.

**3. The state, as a slice plus a lifecycle.** Two columns: the entity slice on the left (a bordered card per entity, listing only the fields that move or gate), the named states on the right as a `stateDiagram-v2` when there are three or more.

Give each named state a colour and **reuse it everywhere on the page** — in the lifecycle diagram, in the state chips inside the trace, and in any legend. That consistency is what makes the deltas readable at a glance instead of needing to be parsed:

```html
<span class="rounded bg-amber-100 px-2 py-0.5 text-xs font-semibold text-amber-800">pending</span>
<span class="rounded bg-emerald-100 px-2 py-0.5 text-xs font-semibold text-emerald-800">accepted</span>
```

Fields render as monospace with their value, `∅` for null. Guard fields get a marker (a lock glyph or a `WHERE` label) so a reader can see which fields are load-bearing for concurrency rather than merely descriptive.

**4. The trace, as a vertical spine.** The centrepiece. One row per hop:

```html
<li class="relative border-l-2 border-slate-200 pl-6 pb-6">
  <span class="absolute -left-[9px] top-1 h-4 w-4 rounded-full bg-slate-300 ring-4 ring-slate-50"></span>
  <code class="text-xs text-indigo-700">backend/src/message/send.rs:248</code>
  <p class="mt-1 text-slate-700">Persists the row with <code>turn_number: None</code> — this is what "pending" means.</p>
</li>
```

The connecting line is the point: it makes the sequence a shape rather than an enumeration, so a reader can see how far through they are without counting. Number the hops as well — the numbers are what someone quotes back at you.

**`file:line` is monospace, accented, and on its own line above the prose.** It is the landmark a reader scans for, so it must be findable without reading the sentence.

**A hop that changes state carries its delta as a chip row**, directly under the prose — same colours as the lifecycle above. A hop that changes nothing carries no chip, which makes the mutating hops findable by scanning:

```html
<div class="mt-2 flex flex-wrap items-center gap-2 text-xs">
  <code class="text-slate-500">turn_number</code>
  <span class="text-slate-400">∅ →</span>
  <code class="font-semibold text-slate-800">4</code>
</div>
```

**5. The diagram — a sequence diagram when the trace crosses a process boundary**, a flowchart when it branches or loops, nothing when it's a straight line. This is where a page beats chat: participants side by side make the "who is waiting on whom" question answerable by looking.

**6. Where it bites, as bordered callouts.** Amber — a different amber from any state chip, or use red-tinted borders if `pending` already owns amber. Three at most. These are the things the trace doesn't show, so they must not look like another hop.

**Colour discipline.** Once named states own colours, the page has a real budget problem: state chips, the accent, and warnings all compete. Assign the states first, then pick the warning treatment around them. Two things that mean different things must never share a colour.

## The seam, when there is one

Two traces need the boundary between them rendered, not just labelled. A full-width band between the two spines — different background, the protocol named, and one line on what does and doesn't cross it.

That band is usually the most valuable element on the page. Most confusion about a distributed path is confusion about what the boundary carries.

## Anchors and links

- Give every hop an `id` (`#hop-3`) so a specific step can be linked in chat or a PR comment.
- **Link `file:line` to GitHub when a remote and a SHA are known** — `https://github.com/<org>/<repo>/blob/<sha>/<path>#L<line>`. Pin the SHA, not the branch: line numbers drift, and a stale deep link is worse than plain text. This is the only form that can anchor a line.
- **Otherwise link into the task UI** — `<origin>/tasks/<task-id>?file=moatless%2F<path>&fileView=code`. No line anchor, but it opens the right file next to the conversation, which beats plain text.
- If neither is available, leave them as plain monospace. Don't guess a URL.

## Keep it honest

- **Every hop on the page was read, and every transport was followed.** The page format makes an unverified claim look more authoritative, which raises the cost of guessing rather than lowering it. Reading a call site tells you a call happens; it does not tell you whether it is HTTP, gRPC or an in-process channel — open the client before naming the transport.
- **Date it in the footer, with the commit SHA** if available. An explanation of code is true as of a revision, and unlike a spec it has no status line to tell the reader otherwise. Since the page lives in `scratchpad/` and is not committed, that stamp is the only thing telling a reader how old it is.

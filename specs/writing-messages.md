# A skill that owns the shape of a message

> **Status:** approved
> **Updated:** 2026-07-27
> **Vocabulary:** CONTEXT.md

## Current solution

Every formatting rule in the skills tree governs an artifact. None governs the turn a person actually reads.

### What each existing rule governs

| Rule | Where | Governs | Read by |
|---|---|---|---|
| "Design against flatness… every section carries a table, list, diagram or code block" | `writing-specs/SKILL.md:37` | page + markdown | human + agent |
| "The page is capped by attention… one screen is what gets approved" | `writing-specs/SKILL.md:35` | the page | human |
| "One question, one screen" | `grilling/SKILL.md:17` | the fenced question block | human |
| "Prefer a link over a paste" | `html-report/SKILL.md:56` | pastes, not narration | human |
| "Under 400 words" | `code-review/SKILL.md:53`, `:59` | a sub-agent brief | **agent** |
| "Wins — bullets, ≤6 words each" | `improve-codebase-architecture/HTML-REPORT.md:33` | page content | human |
| — nothing — | — | **the chat turn** | **human** |

34 skills across `plugins/moatless-skills/skills/` and `.agents/skills/`. Two word caps exist in the whole tree, and both are on text no human reads.

### Prior art already half-imported

`ayghri/i-have-adhd` is an MIT-licensed skill of ten output rules, inspired by *The Adult ADHD Tool Kit* (Ramsay & Rostain). Five are already in this tree, imported piecemeal with no owner.

| `i-have-adhd` | Here | Match |
|---|---|---|
| Override 5: "2 to 4 ranked options with one-line trade-offs, recommendation first" | `grilling:35` — "2–4 options, one line of trade-off each, recommendation first" | verbatim |
| Pre-send 5: idiom — "circle back," "get the ball rolling" | `writing-specs:110` — same two examples | verbatim |
| Rule 5: "The reader cannot hold 'we are on step 3 of 5' between messages. Restate it." | `grilling:33` — "The user cannot hold 'we're on question 3…' between turns. Restate it every time." | near |
| Rule 4: "A question that comes up mid-work is not a tangent: answer it yourself if you can and fold the result in." | `grilling:78–86` — "Park tangents", elaborated into three cases | near |
| Rule 3: "name ONE thing the reader can do in under two minutes. Even 'open the file' counts." | `writing-specs:66` (deleted) — "'Open `backend/src/agent/mod.rs`' counts." | **the bug** |

The last row is the origin of the `Next action: open tests/api/fixtures/api-fixtures.ts` defect. Its rule 3 is correct in its own context — a reader executing steps — and wrong here, where the reader's next act is answering a question.

### The two renderers, which nobody documented as different

| Surface | Renderer | Plugins | Mermaid |
|---|---|---|---|
| Chat messages (`assistant-message.tsx:106` → `response.tsx`) | `Streamdown 1.4.0` | `remark-gfm`, `remark-math`, `rehype-harden/katex/raw` | yes |
| File preview (`markdown-preview.tsx`) | `react-markdown 10` | `remark-gfm` | no |

Neither includes `remark-breaks`. A single newline is a CommonMark **soft break** and renders as a space. Only block-level constructs survive.

Observed live: a three-option block sent as three lines arrived as one paragraph — *"1. The skill body verbatim… ← recommended 2. A directive naming it… 3. Both — body injected…"*. The cause was writing `**1.**`, which is inline emphasis inside a paragraph, not list syntax. A real `1. ` at line start becomes an `<ol>` and renders.

| | Constructs |
|---|---|
| **Survives** | Ordered and bullet lists · headings · tables · code fences · blockquotes · task lists · strikethrough · blank-line paragraph breaks · ` ```mermaid ` |
| **Collapses** | Single newlines inside a paragraph · `**1.**`-style pseudo-lists · anything relying on visual line position |

### Field data — task `747da6d4`, 11:00–11:43

A naming fork was resolved inside a paragraph, at 65% depth of a 470-word turn:

> "So the question becomes: should that upper layer be **Adapter delivery** or **Adapter inbox**? I'd keep Delivery."

It was never offered as a choice.

| Time | Message |
|---|---|
| 11:13 | The turn above |
| 11:17 | "So adapter delivery then?" |
| 11:22 | "a then" |
| 11:28 | "when did we decide that 'Adapter work' stays, I thought we replaced it with Adapter delivery?" |
| 11:31 | "What about renaming the adapter work record to adapter delivery?" |
| 11:40 | "Change to adapter delivery" — 24 files, 355 occurrences |

27 minutes, five user turns. The agent diagnosed it itself at 11:29: *"never as an explicit choice you made… That was my ambiguity to fix, not yours to catch."*

A second turn from the same session, answering "is adapter work more like a queue?", ran ~340 words: a forbidden opener, an eight-item list, a tangent inline, and a closing recap of actions taken.

The example that opened this work measured ~390 words, ask at 67% depth, 50 words after the ask, zero numbered options. One sentence in it carried three findings joined by semicolons — 95 words, violating four COGA patterns at once (4.4.3 nested clauses, 4.4.5 succinct, 4.4.9 separate each instruction, 4.3.1 important tasks findable). It was a three-row table written as prose.

### What each candidate instruction yields on that data

| Instruction | Source | Turn A "is it a queue?" | Turn B "why not inbox?" |
|---|---|---|---|
| No preamble, recap or closer | R10 | −80 words | −60 words |
| Cap lists at 5 | R9 | −25 words | no effect |
| Suppress tangents | R4 | tangent becomes an offer | no effect |
| Restate state every turn | R5 | +15 words | +15, but prevents 11:28 |
| Lead with the answer | R1 | already satisfied | major — verdict was at 30% |
| Strike test | ours | **inapplicable** | **inapplicable** |
| A fork is never resolved in prose | **new** | no fork present | saves 5 turns / 27 min |

Both sampled turns are answers, not questions. With no options, "which option does this push toward" has no referent, so the strike test does not fire on either.

### Evidence the numbered affordance works

In the interview that produced `specs/writing-specs-workflow.md`, every question offering numbered options was answered with a single character: `1`, `1`, `1`, `Yes`. The one question posed as open prose with an embedded if/then drew a paragraph of redirection instead — *"whats the delta to keep?"* — and cost two extra turns.

### Two conflicts already in the tree

| Instruction | Where | Tension |
|---|---|---|
| "Ask the user: does the granularity feel right? / are the blocking edges correct? / should any be merged or split?" | `to-tickets:50–55` | Three questions in one turn. `grilling`'s first always-rule forbids two. |
| "In chat. Always. The answer stands on its own there, whether or not you build anything else." | `explaining-code:122` | Looks like a mandate for a standalone long message. On re-reading it is not — see Decision 6. |

### The workflow mandates a step this change cannot justify

`writing-specs/SKILL.md:14` step 5 is unconditional: *"Run `/to-tickets`. The breakdown lands on the same page and gets its own approval."* Its adjustable knobs let a user stop **earlier**; none lets a change skip the breakdown. A three-file docs change is therefore told to produce a ticket graph that fails `/to-tickets`' own vertical-slice and sizing rules.

### Nothing verifies a skill

No link checker, no eval, nothing in `scripts/` or `.github/workflows/` touches `plugins/moatless-skills/`. That is why `Next action:` survived its own deletion in two files, and why the `/orchestrate` self-contradiction sat through a full rewrite.

## Planned solution

### Problem

The person reading agent turns has ADHD. The load is not length — it is being required to hold things in mind. A 390-word turn that buries its ask at 67% depth forces the reader to carry findings one and two while parsing finding three in order to answer a question about finding one. When the ask is not a numbered option, answering costs a composed paragraph instead of a keystroke. When a fork is resolved inside prose, the reader either misses it or spends five turns recovering it.

The literature is one-sided on the mechanism and mixed on a common misreading of it. Working-memory impairment in ADHD is robust — large effect sizes across verbal and visuospatial domains, affecting storage and central-executive updating alike ([Martinussen et al. 2005](https://www.sciencedirect.com/science/article/abs/pii/S0272735812000979); [Alderson, Kasper et al. 2013](https://pubmed.ncbi.nlm.nih.gov/23688211/)). Sustained-attention decline over time is **not** well supported: [Fuermaier et al. 2015](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC5281679/) found impairment on vigilance tasks but no differential deterioration with time on task. So the rules are not justified by "they will stop reading". They are justified by "nothing should need to be retained".

On form, the general readability evidence is strong and multiplicative: concise text improved measured usability 58%, scannable layout 47%, objective language 27%, and all three together 124% ([Morkes & Nielsen 1997](https://www.nngroup.com/articles/concise-scannable-and-objective-how-to-write-for-the-web/)). Users see roughly two words per item when scanning, so action-carrying terms must be front-loaded ([NN/g](https://www.nngroup.com/articles/first-2-words-a-signal-for-scanning/)); reading follows an F-pattern, first line thoroughly, then left margins ([NN/g, 232 participants](https://www.nngroup.com/articles/f-shaped-pattern-reading-web-content-discovered/)). W3C's cognitive-accessibility note names the specific patterns violated: 4.4.3 avoid nested clauses, 4.4.4 use literal language, 4.4.5 keep text succinct, 4.4.9 separate each instruction, 4.3.1 make important tasks easy to find ([Making Content Usable](https://www.w3.org/TR/coga-usable/)). One thing per page reduces cognitive load and makes the ask unambiguous ([GOV.UK question pages](https://design-system.service.gov.uk/patterns/question-pages/)).

### Behaviour

**Given** the agent is about to send a user-facing turn
**When** it classifies the turn
**Then** it is exactly one of three kinds, and the third is converted to the first.

| Kind | Test | Shape |
|---|---|---|
| Question-turn | Strike test | Ask first, real ordered list, nothing after but one footer line |
| Answer-turn | Verdict first, then only reasoning that could change the verdict | Verdict line, evidence, page link |
| Answer containing a fork | Reclassify as a question-turn | As a question-turn |

**Given** a question-turn
**When** the agent decides which facts go in it
**Then** each fact must earn its place by naming the option it argues for, or what it changes about the cost of picking wrong
**And** a fact that does neither is context, and goes on the page named but not narrated
**And** two roles qualify — evidence (why an option might be right) and stake (what it costs to pick wrong)
**And** nothing else qualifies.

**Given** more than about four facts survive the strike test
**When** the agent notices
**Then** the question is too big and gets split
**And** the message is not trimmed to compensate.

**Given** a question-turn exceeds roughly 150 words
**When** the agent checks before sending
**Then** it re-runs the strike test
**And** it does not satisfy the tripwire by trimming adjectives.

**Given** an answer-turn that resolves a choice
**When** the agent drafts it
**Then** the choice is surfaced as a numbered ask on the first line
**And** it is never stated as a conclusion inside a paragraph.

**Given** the agent writes options
**When** it formats them
**Then** it uses a real `1. ` ordered list at line start
**And** never `**1.**`, which the chat renderer collapses into the surrounding paragraph.

**Given** the agent cuts a finding from a message
**When** it does so
**Then** the finding is on the page and the ask is in the question queue
**And** "not in this message" means later, not gone.

### The new skill, section by section

| Section | Contents |
|---|---|
| Classify the turn | The three kinds; the fork reclassification |
| The question-turn | Strike test; evidence and stake; ask on line 1; real ordered list; nothing after but one footer line; the four-fact split rule; the ~150-word tripwire |
| The answer-turn | Verdict first, then only reasoning that could change the verdict; everything else named, not narrated |
| Six always-rules | R1, R6, R7, R8, R9 with its exception, R10 |
| Formatting that survives | The renderer facts; every structural rule names the construct carrying it |
| Six overrides | Where the rules yield; case 6 reworded for an agent harness |
| Pre-send check | The delete-list, adapted |

### Rules borrowed from `i-have-adhd`

| Src | As written here | Change from the original |
|---|---|---|
| R1 | The ask or the verdict is the first line | "Action" becomes "ask or verdict" — the reader's next act is answering, not executing |
| R6 | Estimate the work in concrete units | Narrowed: never estimate the reading, only the work being proposed |
| R7 | State the new capability, not the actions taken | Reworded so it stops colliding with R10 |
| R8 | Errors are matter-of-fact: cause, then fix | Unchanged |
| R9 | Cap lists at 5, **except sets the reader is choosing from** | New exception, without which the selection-question shape is unaskable |
| R10 | No preamble, no recap, no closing pleasantry | Unchanged. Highest measured yield |

R5 and R10 pull opposite ways and are both right. The line that resolves them: **restate what is settled, cut what you just did.**

R6 and R7 are carried on the general case — `/implement`, `/dev-loop`, `/orchestrate` reporting — not on evidence from the sampled transcripts, where neither fires.

### The overrides

| Case | The rules yield because |
|---|---|
| "Explain" or "walk me through" | Explain fully. Still no preamble, still no closer; add headings so the reader can skim back |
| Destructive action ahead | Confirm before acting. Safety outranks brevity |
| Debug spiral | Three turns of "still broken" — stop iterating, name the assumption that might be wrong, ask one diagnostic question |
| Real ambiguity | One short clarifying question beats guessing and rewriting |
| A rule would delete the answer | The task wins, the shape stays. "What are my options" gets 2–4 ranked options, recommendation first |
| A rule fights the harness | **Reworded, not imported.** Written for an assistant whose system prompt outranks the skill; here the harness is the agent, so "ask before acting" becomes "do the work" |

### Formatting

Every structural rule names the markdown construct that carries it, because the chat renderer has no `remark-breaks` and discards anything relying on line position.

- Options are a real ordered list — `1. ` at line start. Never `**1.**`.
- A blank line is the only reliable separator; single newlines collapse to spaces.
- Three or more parallel items become a GFM table, which is block-level and always survives.
- Diagrams in chat use ` ```mermaid `, which Streamdown renders and the file preview does not.

### `/writing-specs` stops mandating tickets

Step 5 comes out. Tickets do not disappear — they stop being a step of `/writing-specs` and become `/orchestrate`'s job, which is the only caller that spawns work per ticket.

| File | Line | Change |
|---|---|---|
| `writing-specs/SKILL.md` | 14 | Step 5 — "Run `/to-tickets`" — deleted |
| `writing-specs/SKILL.md` | 15, 17, 28, 32, 33, 71, 93 | Step 6 → step 5 throughout; "from step 2 to step 5" → "to step 4" |
| `writing-specs/SKILL.md` | 15, 88 | "work graph included" and `## Ticket breakdown` become conditional |
| `to-spec/SKILL.md` | 21 | "jumps straight to step 6" → step 5 |
| `to-tickets/SKILL.md` | 58, 71 | "Coming from `/writing-specs`… the graph is already on its page from step 4" — no longer how it arrives |
| `orchestrate/SKILL.md` | 25 | `/orchestrate` invokes `/to-tickets` itself, after the spec |
| `writing-specs/HTML-REPORT.md` | 21 | The page's **Work** section becomes optional |
| `specs/writing-specs-workflow.md` | 73–74 | Historical record — superseded note, not a rewrite |

### What changes, in total

| File | Change | Lines |
|---|---|---|
| `writing-messages/SKILL.md` | New. Owns the shape of every user-facing turn | 0 → ~110 |
| `grilling/SKILL.md` | Shape rules out (`10–46`, `78–86`); keeps shared understanding, terminology, batching, the two question shapes; gains a pointer | 90 → ~50 |
| `to-tickets/SKILL.md` | Step 4's three questions become one selection question | −6 |
| `writing-specs/SKILL.md` | Ticket step removed, renumbering | see above |
| `to-spec`, `orchestrate`, `HTML-REPORT.md` | Step-number and routing fixes | small |
| `explaining-code/SKILL.md` | Unchanged — override case 1 covers it | 0 |

### The queue — why cutting is safe

Every rule above removes something from a message, and an agent that finds something good will not delete it on instruction alone. The rules hold only because there are two places for what gets cut: **the page** holds the finding, and **the question queue** holds the ask. "Not in this message" means later, not gone. Without both halves the cutting rules get quietly disobeyed — which is the psychological root of long agent turns, not ignorance of brevity.

## Decisions

| # | Decision | Chosen | Rejected |
|---|---|---|---|
| 1 | `i-have-adhd` rule 3, `Next action:` | Excluded | Include it |
| 2 | Relationship to `i-have-adhd` | Borrow and own | Layer on it as a base; fold into `/grilling` |
| 3 | Markdown formatting | In scope; every structural rule names its construct | Out of scope |
| 4 | Scope of the skill | Every turn, two tests | Question-turns only |
| 5 | Word budget | Soft tripwire, ~150 words on a question-turn | Hard cap; no number at all |
| 6 | The six override cases | Adopted; `/explaining-code` unchanged | Amend `/explaining-code`; neither |
| 7 | `/to-tickets` step 4 | Fixed here as a selection question | Separate change |
| 8 | The six missing rules | All six, adapted | A subset |
| 9 | Verification | Replay the sampled transcripts | A/B eval harness; nothing |
| 10 | Shape rules in `/grilling` | Move them out | Leave them; fold the new skill in |
| 11 | Ticket breakdown for this change | None — and `/writing-specs` stops mandating one | Split into tickets |

### 1. Rule 3 is excluded

`i-have-adhd` rule 3 ends every response with one concrete next action: *"name ONE thing the reader can do in under two minutes. Even 'open the file' counts."* It assumes the reader's next act is executing a step, which is true for a general coding assistant.

Here it is false. In an interview the reader's next act is answering the question, so a file path competes with the ask instead of serving it. This is the defect that shipped as `writing-specs/SKILL.md:66` and produced `Next action: open tests/api/fixtures/api-fixtures.ts` at the end of a finished spec. It was deleted; it does not come back.

### 2. Borrow and own, do not layer

Layering — treating `i-have-adhd` as a base the way `/html-report` is a base — would inherit rule 3 and force this file to negate its own base. That is the exact shape that produced the `/orchestrate` self-contradiction: a document arguing with the thing it builds on. **A base you must argue with is not a base.**

Folding into `/grilling` was also rejected: `/grilling` is loaded only for interviews, and decision 4 puts every turn in scope.

### 3. Formatting is in scope

Discovered live rather than reasoned about. A three-option block sent as three lines arrived as one paragraph because `**1.**` is inline emphasis and single newlines are soft breaks. Neither renderer loads `remark-breaks`.

The consequence is larger than a style note: **"ask first, numbered options, nothing after" is not achievable by layout alone.** Every structural rule must name the markdown construct that carries it, or it is advice the renderer discards.

Mermaid renders in chat but not in the file preview. This does not contradict `/html-report`'s claim that a markdown preview cannot draw diagrams — that claim is about the preview pipeline and stays true there.

### 4. Every turn, two tests

Both sampled turns from `747da6d4` are answers, not questions, and the strike test does not fire on either. Under question-turns-only the skill would have governed neither — and the 27-minute failure happened inside an answer-turn.

Known cost: the answer-turn test is one sentence and has not been validated against transcripts the way the strike test has. Decision 9's replay is what closes that gap.

### 5. A tripwire, not a cap

Turn A went ~340 → ~150 words under the strike test and R10 alone. No budget was applied and none was needed, which is a real argument for having no number at all — `i-have-adhd` has none, and all ten of its rules are structural.

A number was still taken, as a tripwire rather than a target, because "am I over 150" is checkable in a second and "did I apply the strike test honestly" is not. It catches the agent that believes it applied the test and did not.

A hard cap was rejected on the repo's own principle: `/writing-specs` says the stopping test is *sufficiency, not slot-filling*. A cap is satisfied by trimming adjectives — precisely the move the strike test exists to prevent, because it cuts words while keeping the non-load-bearing fact.

### 6. The overrides come in; `/explaining-code` is unchanged

The conflict recorded during research was overstated. `explaining-code:122` says "in chat, always", but `:132` already warns that *"answering in chat and stopping is the common failure"* for a multi-seam trace. The mandate is about not **skipping** the chat answer, not about its length. There is no contradiction about how long it runs.

Override case 1 covers what remains: *"User asks to 'explain' or 'walk me through.' Explain fully. Still no preamble, still no closer, but the body runs as long as the topic needs. Add headers so the reader can skim back."*

The other five overrides matter because rules without stated edges get applied where they do damage. Override 5 is already `grilling:35` verbatim, so it was being relied on by accident; taking it explicitly stops that.

Amending `/explaining-code` was rejected: it is not wrong. It correctly says answer in chat and correctly says the page is the artifact. Changing it to resolve a conflict that does not exist adds a rule that fixes nothing.

### 7. `/to-tickets` step 4

It asks three questions in one turn — granularity, blocking edges, merge-or-split — which `grilling`'s first always-rule forbids. It is already a selection question wearing the wrong clothes: the breakdown is a numbered list of real things, and all three questions ask which of them are wrong.

Fixed here rather than deferred because this spec is the only place the conflict is written down, and both times this session that a rule had no owner it stayed broken for months. The cost is a wider diff and a second reviewer axis.

### 8. All six rules, and R9's exception

R9 caps lists at five. That conflicts with `grilling`'s selection shape — *"here are the eighteen chains that exist, which survive?"* — which is unaskable under a five-item cap. The exception is explicit: **the cap applies to lists the reader skims, not to sets the reader is choosing from.**

R9 does not conflict with `writing-specs:100` ("past seven stories in a group, use a table rather than cutting any") or with "nothing is capped" for the spec markdown. The audience rule already resolves that: capped for humans in chat, uncapped for agents in files.

### 9. Replay, not a harness

Nothing in the repo verifies a skill today. That is not a neutral baseline — it is why `Next action:` survived its own deletion in two files.

Transcript replay is the highest seam available, because a recorded session is the actual production behaviour of a prose skill, and it needs no new infrastructure.

An A/B eval harness on `i-have-adhd`'s model — `cases.jsonl`, `rubric.md`, blind scoring against a baseline with operator configuration isolated so plugins cannot leak into conditions — is real prior art and worth building. It is an eval harness for every skill, not part of writing one.

### 10. Move the shape rules out of `/grilling`

`grilling:10–46` is message shape: the one-question-one-screen block, the progress line, the recommended default, the ranked alternatives, the "you decide" escape. `grilling:78–86` is park-tangents, which is R4.

Leaving them would split the rules by history rather than by concern — the shape that gave the repo a workflow stated in five places and a `Next action:` line outliving its own definition. Re-creating it in a new file on day one was not acceptable.

The cost is real: `/grilling` drops from 90 lines to about 50, and someone reading it alone no longer sees the question-block format that makes it work. It becomes a skill that only makes sense with its pointer followed.

### 11. No tickets, and the workflow stops demanding them

Three coupled files inside one context window fails `/to-tickets`' own sizing rule. Any split would be horizontal — write the skill, edit grilling, edit to-tickets — which the same skill forbids. They are also ordered: pulling shape rules out of `/grilling` before `writing-messages` exists leaves a gap.

That the workflow demanded a breakdown anyway is the finding. `writing-specs/SKILL.md:14` is unconditional and its knobs only allow stopping earlier, never skipping.

`## Ticket breakdown` and the page's Work section stay in the format, filled only when `/to-tickets` actually runs. Removing them was rejected because child tasks read the spec and nothing else — `/orchestrate` inlines `specs/<slug>.md` into every spawn message — so a graph that never reaches the spec never reaches them. The cost is a heading that is empty for most specs, which is a mild version of the unused-slot problem that produced `Next action:`.

## Seams and testing

Nothing in this repo verifies a skill: no link checker, no eval, nothing in `scripts/` or `.github/workflows/` touches `plugins/moatless-skills/`.

The seam chosen is **replay of real transcripts** — the highest available, because a recorded session is the production behaviour of a prose skill.

| Case | Before | Passes when |
|---|---|---|
| Turn A — "is it a queue?" | ~340 words, 8-item list, tangent inline, recap closer | Verdict on line 1; tangent becomes a separate offer; no recap |
| Turn B — "why not inbox?" | ~470 words, fork resolved in prose at 65% depth | Fork surfaced as an ordered list on line 1; answerable with `1` |
| The pasted example | ~390 words, ask at 67%, 50 words after it, 0 options | Ask first, nothing after it but one footer line |
| Any option block | `**1.**` collapsed to one paragraph | Renders as an `<ol>` in Streamdown — real `1. ` at line start |

## Out of scope

- **How the skill is delivered.** No toggle, no system-prompt injection. Research parked below.
- **Page layout** — `/html-report` and `writing-specs/HTML-REPORT.md` own it.
- **Spec markdown shape** — uncapped by design, `/writing-specs` owns it.
- **Sub-agent briefs** — not read by a human.
- **Which question to ask** — `/grilling` keeps that.
- **An A/B eval harness** — worth building, for every skill, separately.

### Parked: the always-on delivery research

Investigated, then ruled out of scope. Kept for whoever picks it up.

`plugin_activations (plugin_id, skill_name, user_id, enabled)` already exists (migration `0102`). `backend/src/plugin/activation.rs` resolves it on two axes with documented precedence — Personal beats Everyone, and a skill-specific record beats a plugin-wide one, so "the whole plugin except this one skill" is already expressible. Resolution is pure and unit-tested.

It controls one thing: whether a skill's files are **staged** into the sandbox (`sandbox/src/init/plugin_skills.rs`). Staged means invocable, not present. Nothing puts skill content into `build_append_system_prompt` (`backend/src/agent/prompt.rs:201`), which today assembles task identity, the workspace `system_prompt`, the task's `append_system_prompt`, servers, task URL, parent, and two static constants.

So an always-on toggle would be one flag on an existing record plus one section in an existing builder. Two workspace-level prompt fields already prove the injection path works.

Separately: nothing in `.agents/skills/` references a plugin file by path. The only crossing between the two skill trees is by slash-name, so rules that must reach both have to be a named skill rather than a reference file.

## Open questions

- **The idiom rule now lives in two places.** `writing-specs:110` bans "circle back" and "get the ball rolling" for spec prose; the pre-send check bans them for messages. Different surfaces, so it may be legitimate — but it is the same duplication pattern this spec exists to fix.
- **Nothing carries the rules to skills outside the interview path.** Delivery is out of scope, so `/implement`, `/orchestrate` and `/diagnosing-bugs` get the rules only when the skill is invoked.
- **The answer-turn test is one sentence.** The strike test was validated against two real transcripts; its counterpart was not.
- **`## Ticket breakdown` is now empty for most specs.** A permanently unused slot is the mild form of the failure that produced `Next action:`.

## Ticket breakdown

None. See decision 11.

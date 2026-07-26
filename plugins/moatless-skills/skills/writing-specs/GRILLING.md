# Grilling toward a lede

Base discipline: the `/grilling` skill — one question at a time, wait for each answer, look facts up rather than asking, don't act until the user confirms you've reached shared understanding. This file adds the layer that binds it to the lede.

## Bound the interview to the lede's slots

Open-ended grilling is where attention dies: the user can't see how many questions are left, so every answer feels like it might not be the last, and the session ends by exhaustion rather than by agreement.

The lede has a fixed number of slots. Bind the question set to them and the interview gets a visible end.

| Lede slot | What it asks |
|---|---|
| *(none — `## Today`)* | "Is this what we're both looking at?" The understanding batch, below. Asked before any slot gets filled. |
| **After this ships** | "What can someone do afterward that they can't today?" Ask this first of the slots, once. Everything else hangs off the answer. |
| **In scope / Out of scope** | One question per contested edge — not per bullet. Uncontested bullets you write yourself. |
| **Decisions** | One question per decision that is genuinely open. This is the bulk of the interview. |
| **Open questions** | Never asked. This is where questions you *can't* resolve get parked. |
| **Work** | Never asked. `/to-tickets` derives it. |
| **Next action** | Never asked. You write it. |

Count the open slots before the first question and say the number: *"About 6 questions — 1 on scope, 4 decisions, 1 boundary."* An estimate the user can hold is worth more than an accurate count they can't see.

## Open with the understanding batch

Before any slot gets filled, spend **1–3 questions** establishing that you and the user are describing the same system in the same words. This is the batch that pays for itself: a decision made on a mismatched picture is not a slow decision, it is a wrong one, and it comes back as rework after the spec is approved.

Two questions, at most three:

**1. Does `## Today` match your model?** Link the section — don't paste it — and name the one or two findings that will actually move the spec:

> `## Today` is written: [spec](…#today). Two things worth flagging before we start.
>
> - There are **two** delivery paths and only one runs; the dead one is where the second vocabulary comes from.
> - Nothing persists a skill, so a skill has no id to hang anything off.
>
> Does that match how you thought it worked, or is something missing?

The point is not approval of the section. It is finding the place where their model and the code disagree, because that gap will otherwise surface as a rejected decision five questions later. **A "no, actually…" here is the cheapest correction in the whole workflow.**

**2. Do the terms mean the same thing to both of us?** Check the affected area's vocabulary against `CONTEXT.md` before asking anything phrased in it. Three cases, one line each:

| | Do |
|---|---|
| Term is in `CONTEXT.md` and the code agrees | Nothing. Use it |
| Term is in `CONTEXT.md` and the code disagrees | Say so — that's a finding, and often the spec's actual problem |
| Term isn't there at all | Propose a definition, confirm it, land it in `CONTEXT.md` (`/domain-modeling`) |

Cap the list at the terms the spec will actually use. This is not a glossary audit — it is making sure the decision questions land.

**When the change *is* a rename, this batch is not optional.** A spec that aligns terminology while the glossary stays silent on those terms has no anchor to align to, and every decision below it is a preference rather than a consequence.

Skip the batch when the user wrote the code themselves, or when `## Today` fit in five lines. Say you're skipping it and why.

## Run one batch at a time

The slots also set the order: understanding, then scope, then decisions — never alternate. Attention here runs deep and narrow; being pulled out of a subject costs far more than being asked one more question inside it, so an interview that hops between scope and seams and rollout is expensive in a way its question count doesn't show.

Open each batch by naming it and its size:

> **Decisions — 4 questions:** the seam, the retry policy, message assembly, rollout. Starting with the seam.

That is also the moment to offer the shape:

> One at a time, or do you want all four at once?

Ask it once per interview, at the first batch, and then honour the answer without asking again. One at a time is the default because it's the safe one; a session with momentum often wants the whole batch, and only the user knows which one this is.

Inside a batch, still one question per turn.

## One question, one screen

```
**Decisions 2 of 4** (Q3 of ~6) · settled so far: in-process seam, no new crate

Where does the retry policy live — inside the harness, or in the coordinator?

1. **Coordinator** — one policy for every harness; harness only classifies errors   ← recommended
2. Harness — each CLI retries its own way; risks two divergent policies
3. Both — harness retries transport, coordinator retries semantics; more moving parts

Answer with a number, your own answer, or "you decide".
```

Four things are doing work in that block, and each maps to a fact about how this gets read:

1. **The progress line.** The user cannot hold "we're on question 3 and we already settled the seam" between turns. Restate it every time — position within the batch, position overall, and what is already decided. Never ask them to remember an earlier answer.
2. **A recommended default.** Reacting to a proposal is far cheaper than generating one from a blank prompt, and the gap between knowing and doing is where the session stalls. Always lead with your recommendation; never ask a bare open question.
3. **Ranked alternatives, not one path.** For a decision, the alternatives *are* the thing the user has to hold — collapsing to a single option doesn't reduce load, it hides the choice. 2–4 options, one line of trade-off each, recommendation first.
4. **The "you decide" escape.** A decision the user can't make is where the session dies. Let them hand it back, then record what you chose in the decision row, so it stays visible and reversible instead of silently becoming fact.

## Two surfaces: the question in chat, the detail in the file

The loop is short on purpose:

1. Answer lands → write it into `specs/<slug>.md`.
2. Ask the next question.

**Nothing is rendered between those two steps.** A page that has to be rebuilt after every answer puts a build step where the next question should be, and the user is left waiting on a tab they aren't looking at — they are looking at the chat, because that is where the question is.

So the two surfaces divide cleanly:

| | Carries | Never carries |
|---|---|---|
| **The chat message** | The question, its ranked options, the progress line | Background the file already holds |
| **`specs/<slug>.md`** | `## Today`, every settled answer, everything parked | The current question |

The question is short because the file is one click away. When a question needs current behaviour to make sense, **link the section and name the one fact that decides it** — don't re-explain the mechanism inside the question. Pasting three paragraphs of background above a decision means the reader spends their attention on the background and arrives at the options with none left.

Never let an answer live only in the chat. File first: a restarted sandbox or a closed session then costs nothing, and the interview can stop after any question and resume from the markdown.

The rendered page comes once, at approval — see [HTML-REPORT.md](HTML-REPORT.md).

## Look it up; only ask decisions

If the answer is a fact — what the code does today, which crate owns a type, whether a test exists — find it. Only decisions get asked. Nothing burns attention faster than being asked something the agent could have read.

## Park tangents, don't chase them

A question that surfaces mid-interview is the work happening, not clutter — but it doesn't get asked here.

- **You can answer it** → answer it, fold the result into the current question, say nothing about the detour.
- **It needs the user but isn't blocking** → write it into the lede's **Open questions** and move on. It gets surfaced once, in the artifact, where it survives the session.
- **It blocks the current question** → it *becomes* the current question. Ask it instead, not as well.

Never stack two questions in one turn unless the user asked for the whole batch. Two unrequested questions means they pick which to answer, and picking is where work stalls — a batch they opted into is a different thing, because the choosing already happened.

## Stop when every slot is filled or parked

Then write the lede and show it. That artifact *is* the shared-understanding check `/grilling` asks for — an agreement you can both point at beats an agreement you both remember.

Record any decision that is hard to reverse, surprising without context, and the result of a real trade-off as an ADR while it's fresh (`/domain-modeling`). Sharpen fuzzy terms into `CONTEXT.md` the moment they're settled — don't batch them to the end.

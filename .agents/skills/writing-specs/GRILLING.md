# Grilling toward a lede

Base discipline: the `/grilling` skill — one question at a time, wait for each answer, look facts up rather than asking, don't act until the user confirms you've reached shared understanding. This file adds the layer that binds it to the lede.

## Bound the interview to the lede's slots

Open-ended grilling is where attention dies: the user can't see how many questions are left, so every answer feels like it might not be the last, and the session ends by exhaustion rather than by agreement.

The lede has a fixed number of slots. Bind the question set to them and the interview gets a visible end.

| Lede slot | What it asks |
|---|---|
| **After this ships** | "What can someone do afterward that they can't today?" Ask this first, once. Everything else hangs off the answer. |
| **In scope / Out of scope** | One question per contested edge — not per bullet. Uncontested bullets you write yourself. |
| **Decisions** | One question per decision that is genuinely open. This is the bulk of the interview. |
| **Open questions** | Never asked. This is where questions you *can't* resolve get parked. |
| **Work** | Never asked. `/to-tickets` derives it. |
| **Next action** | Never asked. You write it. |

Count the open slots before the first question and say the number: *"About 6 questions — 1 on scope, 4 decisions, 1 boundary."* An estimate the user can hold is worth more than an accurate count they can't see.

## Run one batch at a time

The slots also set the order. Ask every scope question, then every decision — never alternate. Attention here runs deep and narrow; being pulled out of a subject costs far more than being asked one more question inside it, so an interview that hops between scope and seams and rollout is expensive in a way its question count doesn't show.

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

## Put the board on screen

For an interview past about four questions, run a live **board** alongside it — the lede rendered as a skeleton, empty slots and all, filling as answers land. See [HTML-REPORT.md](HTML-REPORT.md) for the page; what matters to the interview is the loop:

1. Answer lands → write it into `specs/<slug>.md`.
2. Re-render the board from the file, **with the Write tool** — that tool call is what tells the user's open preview tab to refetch. A shell redirect updates the file and leaves the tab stale.
3. Ask the next question.

The board carries the state so the chat doesn't have to, which is the one thing that makes a long interview survivable: every settled answer stays permanently on screen instead of scrolling away, and the empty slots show how much is left without anyone counting.

The progress line stays in the chat question regardless — the board can be closed, minimised, or on another monitor.

Never let an answer live only on the board. File first, render second: a closed tab or a restarted sandbox then costs nothing, and the interview can stop after any question and resume from the markdown.

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

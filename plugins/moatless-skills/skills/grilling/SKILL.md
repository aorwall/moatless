---
name: grilling
description: Grill the user relentlessly about a plan, decision, or idea. Use when the user wants to stress-test their thinking, or uses any 'grill' trigger phrases.
---

# Grilling

Interview the user relentlessly about every aspect of this until you reach a shared understanding. Walk down each branch of the decision tree, resolving dependencies between decisions one by one.

Four rules, always:

- **One question per turn**, and wait for the answer. Two questions means the user picks which to answer, and picking is where work stalls.
- **Every question carries your recommended answer.** Reacting to a proposal is cheap; answering a blank prompt is not.
- **Look facts up; only ask decisions.** If it can be found in the filesystem or with a tool, find it. Nothing burns attention faster than being asked something you could have read.
- **Don't act until the user confirms** you have reached shared understanding.

**`/writing-messages` owns the shape of the turn** — where the ask sits, how the options are formatted so they render, what earns a place in the message and what goes on the page instead. This skill owns *which question to ask*. Read both; neither restates the other.

Two things here that the shape rules assume and don't supply:

- **A "you decide" escape on every question.** A decision the user can't make is where the session dies. Let them hand it back, then record what you chose so it stays visible and reversible instead of silently becoming fact.
- **The progress line names what is already settled**, not just the position. The user cannot hold "we're on question 3 and we already agreed the seam" between turns, and being asked to re-derive it is how a session stalls.

## Two question shapes, and the second one is easy to miss

**A ranked pick** — 2–4 options, recommendation first, laid out as `/writing-messages` describes. Right when the answer is genuinely one option out of a few: which language, where the seam sits, build it now or later.

**A selection from a set** — right when the answer is "which of these real things survive, and where does each go". Show the set: every item, one line each, with what you'd do with it. Then ask which survive.

**If you are about to bundle a list of real things into 2–4 combinations, you want the second shape.** The bundles are your guesses at groupings, and the user will regroup in ways you didn't offer — so the bundles hide the items. "Which of these four tiers run in CI" beats "db+sandbox / all / db only / sandbox+agent". "Here are the eight things only this file says, keep or move" beats "keep it / trim it / delete it".

The tell: you find yourself writing an option that means "some of them".

## Reach shared understanding before deciding anything

Spend the first one to three questions establishing that you and the user are describing the same system in the same words. A decision made on a mismatched picture is not a slow decision, it is a wrong one, and it comes back as rework after everything is agreed.

**Does your reading of the current state match theirs?** Name the one or two findings that will actually move the outcome — not a summary. The point is not approval; it is finding the place where their model and the code disagree. **A "no, actually…" here is the cheapest correction in the whole session.**

**Do the terms mean the same thing to both of you?** Check the affected area's vocabulary against the project's glossary before asking anything phrased in it:

| | Do |
|---|---|
| Term is in the glossary and the code agrees | Nothing. Use it |
| Term is in the glossary and the code disagrees | Say so — that's a finding, and often the actual problem |
| Term isn't there at all | Propose a definition, confirm it, land it in the glossary |

Cap the list at terms the work will actually use. This is not a glossary audit. **When the change *is* a rename, this is not optional** — aligning terminology while the glossary stays silent leaves nothing to align to.

Skip it when the user wrote the code themselves. Say you're skipping it and why.

## Run one batch at a time

Group questions by subject and finish a batch before opening the next. Attention runs deep and narrow; being pulled out of a subject costs far more than being asked one more question inside it, so a session that hops between scope and seams and rollout is expensive in a way its question count doesn't show.

Open each batch by naming it and its size:

> **Decisions — 4 questions:** the seam, the retry policy, message assembly, rollout. Starting with the seam.

That is also the moment to offer the shape — *"one at a time, or all four at once?"* Ask once, at the first batch, then honour the answer. One at a time is the default because it's the safe one; a session with momentum often wants the whole batch, and only the user knows which one this is.

Say the estimated total count up front. An estimate the user can hold is worth more than an accurate count they can't see.

## Park tangents, don't chase them

A question that surfaces mid-interview is the work happening, not clutter — but it doesn't get asked here.

- **You can answer it** → answer it, fold the result into the current question, say nothing about the detour.
- **It needs the user but isn't blocking** → park it as an open question and move on. It gets surfaced once, where they will see it before agreeing.
- **It blocks the current question** → it *becomes* the current question. Ask it instead, not as well.

The parked list is what makes the first two safe. Cutting a question from this turn means later, not gone — and an interviewer who doesn't believe that starts stacking questions to avoid losing them.

## If the outcome is a spec or a plan

Run the `/writing-specs` skill instead. It owns the workflow — what to research, when to render the page, when the markdown gets written — and applies everything above inside it.

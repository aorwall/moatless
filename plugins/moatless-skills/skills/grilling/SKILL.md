---
name: grilling
description: Grill the user relentlessly about a plan, decision, or idea. Use when the user wants to stress-test their thinking, or uses any 'grill' trigger phrases.
---

# Grilling

Interview the user relentlessly about every aspect of this until you reach a shared understanding. Walk down each branch of the decision tree, resolving dependencies between decisions one by one.

Four rules, always:

- **One question per turn**, and wait for the answer. Two questions means the user picks which to answer, and picking is where work stalls.
- **Look facts up; only ask decisions.** If it can be found in the filesystem or with a tool, find it.
- **A "you decide" escape on every decision.** Let the user hand a decision back, then record what you chose so it stays visible and reversible. Approval is theirs alone and takes no escape.
- **Don't act until the user confirms** you have reached shared understanding.

**Run `/asking-questions` for every question you send, and `/writing-messages` for every turn.** The first owns the two question shapes and what earns a place in the message; the second owns everything true of any turn a person reads. This skill owns *which* question to ask, in what order, and when to stop.

## Reach shared understanding before deciding anything

Spend the first one to three questions establishing that you and the user are describing the same system in the same words.

**Does your reading of the current state match theirs?** Name the one or two findings that will actually move the outcome — not a summary. The point is not approval; it is finding the place where their model and the code disagree.

**Do the terms mean the same thing to both of you?** Check the affected area's vocabulary against the project's glossary before asking anything phrased in it:

| | Do |
|---|---|
| Term is in the glossary and the code agrees | Nothing. Use it |
| Term is in the glossary and the code disagrees | Say so — that's a finding, and often the actual problem |
| Term isn't there at all | Propose a definition, confirm it, land it in the glossary |

Cap the list at terms the work will actually use. This is not a glossary audit. **When the change *is* a rename, this is not optional** — aligning terminology while the glossary stays silent leaves nothing to align to.

Skip it when the user wrote the code themselves. Say you're skipping it and why.

## Run one batch at a time

Group questions by subject and finish a batch before opening the next. Being pulled out of a subject costs the user far more than one more question inside it.

Open each batch by naming it and its size:

> **Decisions — 4 questions:** the seam, the retry policy, message assembly, rollout. Starting with the seam.

That is also the moment to offer the shape — *"one at a time, or all four at once?"* Ask once, at the first batch, then honour the answer. One at a time is the default because it's the safe one; a session with momentum often wants the whole batch, and only the user knows which one this is.

Say the estimated total count up front. An estimate is enough — don't delay the batch to make it accurate.

## Park tangents, don't chase them

A question that surfaces mid-interview doesn't get asked here.

- **You can answer it** → answer it, fold the result into the current question, say nothing about the detour.
- **It needs the user but isn't blocking** → park it as an open question and move on. It gets surfaced once, where they will see it before agreeing.
- **It blocks the current question** → it *becomes* the current question. Ask it instead, not as well.

The parked list is what makes the first two safe: cut from this turn means later, not gone.

## If the outcome is a spec or a plan

Run the `/spec` skill instead. It owns the workflow — what to research, when to render the page, when the markdown gets written — and applies everything above inside it.

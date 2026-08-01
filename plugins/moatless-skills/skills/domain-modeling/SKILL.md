---
name: domain-modeling
description: Build and sharpen a project's domain model. Use when the user wants to pin down domain terminology or a ubiquitous language, record an architectural decision, or when another skill needs to maintain the domain model.
---

# Domain Modeling

Actively build and sharpen the project's domain model as you design. This is the *active* discipline — challenging terms, inventing edge-case scenarios, and writing the glossary and decisions down the moment they crystallise. (Merely *reading* `CONTEXT.md` for vocabulary is not this skill — that's a one-line habit any skill can do. This skill is for when you're changing the model, not just consuming it.)

## File structure

Most repos have a single context:

```
/
├── CONTEXT.md
└── src/
```

If a `CONTEXT-MAP.md` exists at the root, the repo has multiple contexts — see [CONTEXT-FORMAT.md](./CONTEXT-FORMAT.md) for the map format and per-context layout.

Create files lazily — only when you have something to write. If no `CONTEXT.md` exists, create one when the first term is resolved.

## During the session

Every question below is a ranked pick between two readings of a term. Send it per `/asking-questions` — the ask first, the readings as a real numbered list, your recommendation marked.

### Challenge against the glossary

When the user uses a term that conflicts with the existing language in `CONTEXT.md`, call it out immediately. "Your glossary defines 'cancellation' as X, but you seem to mean Y — which is it?"

### Sharpen fuzzy language

When the user uses vague or overloaded terms, propose a precise canonical term. "You're saying 'account' — do you mean the Customer or the User? Those are different things."

### Discuss concrete scenarios

When domain relationships are being discussed, stress-test them with specific scenarios. Invent scenarios that probe edge cases and force the user to be precise about the boundaries between concepts.

### Cross-reference with code

When the user states how something works, check whether the code agrees. If you find a contradiction, surface it: "Your code cancels entire Orders, but you just said partial cancellation is possible — which is right?"

### Update CONTEXT.md inline

When a term is resolved, update `CONTEXT.md` right there. Don't batch these up — capture them as they happen. Use the format in [CONTEXT-FORMAT.md](./CONTEXT-FORMAT.md).

`CONTEXT.md` should be totally devoid of implementation details. Do not treat `CONTEXT.md` as a spec, a scratch pad, or a repository for implementation decisions. It is a glossary and nothing else.

### Record decisions where they are active

Do not create a separate architecture-decision log for the repo.

Route durable decisions to the smallest artifact that current work already owns:

- In-flight change decisions go in the active `.plans/<slug>.md` Decisions table, but only for the agent implementing or reviewing that plan.
- Domain vocabulary goes in `CONTEXT.md`; implementation decisions do not.
- Durable implementation guidelines and decisions go in the nearest relevant `AGENTS.md`.
- Shipped behaviour belongs in current code and tests. Do not recreate a standalone markdown specs tree for it.

Plans and old decision records can rot. Treat them as leads, not authority; verify durable architecture and behaviour claims against current code and tests before relying on them.

If none of those fits, ask before adding a new maintained place.

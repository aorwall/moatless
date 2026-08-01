---
name: write-skill
description: Create, revise, package, and prune agent skills for predictable use across Claude Code and Codex-style runtimes. Use when creating a new skill, replacing or merging skills, tuning invocation, editing skill metadata, or when another skill needs skill-writing guidance.
---

# Write Skill

Write skills that make an agent follow a predictable process. Predictable means the same kind of work happens every run; the output can still vary when the task calls for judgment.

Do the smallest useful loop for the user's request: create or edit the skill, validate the package, and use the lightest behavior check that can catch the defect this change could introduce. Use a formal eval suite when the user asks for one or the change needs repeatable behavioral comparison.

## Agent-Only Content

A skill is runtime instruction for an AI agent. It is not a human guide, launch note, explanation of why the skill exists, or record of the conversation that produced it.

Every line in `SKILL.md` must change what the agent does when the skill runs. Keep:

- Steps the agent follows.
- Conditions for when to branch or stop.
- Rules, definitions, examples, formats, and constraints the agent applies.
- Pointers to reference files the agent may need.

Prune content that fits these descriptions:

- Human-facing overview text.
- Motivational language.
- Creation history.
- Installation or usage notes for people.
- Explanations that help a human browse the skill but do not change agent behavior.
- Duplicate summaries of reference files.

## Process

### 1. Capture intent

Start from concrete uses, not from a title.

Establish:

- What the skill enables the agent to do.
- Which user prompts, files, repo contexts, or other skills should reach it.
- Whether it should be model-invoked, user-invoked, or orchestrator-invoked.
- What output or state proves the run is complete.
- Which parts are deterministic enough to deserve bundled commands, scripts, templates, or reference files.

Ask only for decisions. Inspect local files, existing skills, docs, and examples yourself.

### 2. Pick invocation deliberately

Invocation has two separate axes:

| Axis | Meaning | Use |
|---|---|---|
| Implicit model invocation | The runtime advertises the skill so the model can choose it from the user's prompt. | Skills the model should discover on its own. |
| Explicit invocation | A user, router, orchestrator, or already-running skill names the skill intentionally. | Commands, sensitive workflows, expensive workflows, or subskills used only inside a larger plan. |

Some skills should be explicitly usable by an orchestrator but never auto-selected from an ordinary user prompt. For those, keep the skill callable by name, but disable implicit invocation in every supported runtime.

For Moatless plugin skills, set both controls for skills that must not be model-invoked:

```yaml
---
name: example-skill
description: Short explicit-use summary.
disable-model-invocation: true
---
```

```yaml
policy:
  allow_implicit_invocation: false
```

The first block is for Claude-style runtimes that read skill frontmatter. The second belongs in `agents/openai.yaml` for Codex/OpenAI-style runtimes. Keep both when the intent is "callable explicitly, not auto-invoked."

For model-invoked skills:

- Omit `disable-model-invocation`.
- Omit `policy.allow_implicit_invocation: false`.
- Write a model-facing description with the leading word first and one trigger per branch.

### 3. Shape the package

Use this folder shape:

```text
skill-name/
├── SKILL.md
├── agents/
│   └── openai.yaml
├── scripts/
├── references/
└── assets/
```

Only create directories that earn their place.

- `SKILL.md` contains the steps and the reference every run needs.
- `agents/openai.yaml` contains UI metadata and Codex/OpenAI policy.
- `scripts/` contains deterministic or repeatedly rewritten commands.
- `references/` contains material loaded only for some branches.
- `assets/` contains templates or files used in the output.

Keep human-facing readme, installation, changelog, and process-history material outside the skill package. Extra narrative files create a second place for meaning to drift.

Write skill-body content for runtime behavior. UI labels, marketplace summaries, and invocation chips belong in runtime metadata such as `agents/openai.yaml`; operational behavior belongs in `SKILL.md`.

### 4. Write the description

The description is the top-level context pointer for model-invoked skills. Every word in it competes with every other skill.

For model-invoked skills:

- Front-load the leading word.
- State what the skill does.
- Add one trigger per real branch.
- Add "or when another skill needs ..." only if other skills should discover it.
- Put workflow details in the body, not in the description.

For user-invoked or orchestrator-invoked skills:

- Keep the description to a short explicit-use summary.
- Strip broad trigger lists.
- Make the explicit caller responsible for deciding when to use it.

### 5. Build the information hierarchy

A skill has two content types:

- **Steps**: ordered actions the agent performs.
- **Reference**: definitions, rules, examples, formats, or branch-specific details the agent consults.

Keep every-run steps in `SKILL.md`. Move branch-specific or bulky reference behind a clearly worded pointer to a sibling file. The pointer text decides whether the agent loads it reliably; if must-have material is missed, sharpen the pointer before inlining the file.

Co-locate a concept's definition, rules, examples, and caveats under one heading. Scattered facts make the agent rediscover the concept every run.

### 6. Write behavior-forward instructions

Prefer positive target behavior. For style, communication shape, workflow preference, and output structure, describe what the agent should do instead of naming the unwanted behavior. This keeps the useful behavior salient and avoids teaching the agent a bad pattern by repeating it.

Use explicit prohibitions only for hard guardrails: destructive actions, security boundaries, privacy constraints, irreversible workflow changes, or runtime policies where violating the rule would cause concrete harm. Pair the guardrail with the expected behavior.

### 7. Write steps with completion criteria

Every step needs a checkable stopping condition. A good criterion says how the agent knows the work is done, not just what topic was handled.

Prefer:

- "Done when every modified model has a migration, API contract, and caller accounted for."
- "Done when one command proves the bug can go red on the user's exact symptom."
- "Done when the page exists, the approval decision is visible, and the link is in the message."

Weak criteria:

- "Understand the current state."
- "Make the implementation good."
- "Produce a useful summary."

Completion criteria control legwork. If a step repeatedly finishes too early, sharpen the criterion before splitting the skill.

### 8. Use leading words

A leading word is a compact concept the model already knows and can think with: tracer bullet, seam, tight loop, red-capable, single source of truth.

Use leading words to collapse repeated explanation. The word should carry behavior, not branding. If you need a paragraph every time the word appears, the word is not earning its place.

The same leading word should appear in prompts, docs, skill descriptions, and codebase vocabulary when it is meant to drive invocation.

### 9. Prune hard

Read every sentence as a behavior claim.

Prune it when:

- It says what the agent would already do.
- It repeats a meaning that has a single source elsewhere.
- It describes the creation history rather than runtime behavior.
- It explains the skill to a human instead of instructing the agent.
- It names an unwanted behavior where a positive target would make the useful behavior clearer.
- It documents a branch that only some runs need and can move behind a pointer.

Reserve explicit prohibitions for hard guardrails, and pair them with the positive behavior.

### 10. Validate and forward-test

Run the available skill validator or schema check for the target runtime. If the validator is unavailable, manually check:

- Folder name matches `name`.
- `SKILL.md` has valid frontmatter.
- `description` matches the invocation choice.
- `agents/openai.yaml` exists for Moatless plugin skills.
- Implicit-invocation policy is set consistently for Claude-style and Codex/OpenAI-style runtimes.
- Every referenced file exists and is linked from `SKILL.md`.
- No removed or renamed skill is still referenced.

Forward-test when the skill is new, broad, sensitive, or changed in a way that could alter behavior. Use a fresh agent or fresh context if available. Give it the skill and a realistic task, not your expected answer. Review whether the skill changed the process, not whether the final wording sounds nice.

### 11. Present the change

Report:

- Which skills or files changed.
- Which invocation mode each changed skill now has.
- Which checks passed.
- Any validation or forward-test you skipped and why.

## Skill Types

### Model-invoked skill

Use when the agent should discover the skill from ordinary user prompts.

It needs:

- Concise trigger-rich description.
- No implicit-invocation disable flags.
- Clear branch pointers when it supports multiple paths.

### User-invoked skill

Use when the human should decide to run it.

It needs:

- Short explicit-use description.
- `disable-model-invocation: true`.
- `agents/openai.yaml` with `policy.allow_implicit_invocation: false`.
- A body that assumes the user already chose it.

### Orchestrator-invoked skill

Use when another workflow may name the skill deliberately, but the model should not auto-select it from normal prompts.

It needs the same invocation guards as a user-invoked skill. The orchestrator owns the routing decision and names the skill explicitly in its own steps.

### Reference skill

Use when the skill mostly supplies vocabulary or rules. It may have no ordered steps. It still needs a completion criterion for the consuming work: "done when every applicable rule has been checked" is valid for flat reference.

### Shortcut skill

Prefer folding skill composition into the caller. A shortcut skill is acceptable when it removes real cognitive load and has a stable explicit caller, or when the shortcut is a user-facing command worth remembering.

## Moatless Plugin Defaults

For plugin skills in `plugins/moatless-skills/skills`:

- Add `agents/openai.yaml` for every skill.
- Use `interface.display_name` and `interface.short_description`.
- Add `policy.allow_implicit_invocation: false` whenever `disable-model-invocation: true` appears in `SKILL.md`.
- Keep model-invoked skills discoverable only when the model should pick them without a router.
- Keep user-invoked and orchestrator-invoked skills explicit-only on both Claude-style and Codex/OpenAI-style runtimes.
- Prefer sibling Markdown files over nested reference chains.
- Keep `SKILL.md` below 500 lines unless there is a clear reason.

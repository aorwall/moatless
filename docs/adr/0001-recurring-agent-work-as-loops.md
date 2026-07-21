# Recurring agent work is modeled as Loops

Recurring, automated agent work is a **Loop** — a standing, named definition that
fires on a **Schedule** (time-based) or a **Subscription** (to an Adapter's
events) and drives one or more Tasks. Manual one-off Tasks are created directly
and are not Loops. "Trigger" is a verb only — an event *triggers* a Loop; there
is no Trigger noun.

## Considered options

The alternative was a **Trigger-centric** model: a `Trigger` creates Tasks,
composed of a routing `Rule` plus a `Source` (adapter-event or schedule), with a
`Loop` as a narrow `LOOP.md` authoring format that "reconciled into" a Trigger.
Rejected because it inverts the natural hierarchy — the recurring-work concept
became subordinate to its own firing mechanism — and it overloads "Trigger" at
two levels at once (the umbrella `TriggerResponse` and the adapter-event source
struct `TaskTrigger`).

## Consequences

The schema carries rename debt: `Trigger`/`TriggerResponse` → Loop;
`TriggerSource`/`TaskTrigger`/`ScheduledJob` → Schedule/Subscription;
`TriggerRule`/`TaskRoute` → routing. The tables `trigger_rules`,
`adapter_event_trigger_sources`, and `schedule_trigger_sources` keep their legacy
names until migrated. `TriggerKind::Manual` disappears — manual creation is not a
Loop.

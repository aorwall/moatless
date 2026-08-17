# crates/loops

The loops band: Adapter vocabulary, durable inbound delivery, bindings, and
loop sync. It sits below the backend and above the provider crates
(`adapter-linear`, `adapter-github-pr`, …), which is why both ends can name it.

## Two seams join a provider to the rest of the system

Neither is a match arm, and that is the point: adding a provider adds a module
and one registration line, and never edits an arm another provider is also
editing.

- `adapters::delivery` — **inbound, durable**. A webhook handler admits work
  under a `kind`; the worker executes the handler registered for the same
  `kind`. At-least-once, so every handler must be idempotent.
- `adapters::events` — **outbound, best effort**. A provider subscribes a
  `TaskEventSubscriber` to the `EventKey`s it reacts to; the backend's
  event-handler consumer fans each task event out to them.

Put durable provider work behind `delivery`. Put "mirror what is happening back
to the provider" behind `events`.

## Rules for a `TaskEventSubscriber`

- **Keep `on_task_event` cheap.** It runs inline on the consumer's dispatch, one
  event at a time. Decide there, and `tokio::spawn` the provider call.
- **That spawn stays detached.** A progress notification lost to a restart is
  lost on purpose — the run is unaffected and the next event carries the task's
  state again — so there is nothing for a supervisor to own. Work that must not
  be lost is a delivery, not a subscription.
- **Own the whole reaction, including its ordering.** If a provider keeps
  per-task state (Linear's turn gate), the subscriber owns it and both the
  emission and the state change happen there, in the order the provider needs.
  Splitting the two across bands is how the gate came to close before the
  activity it was gating was emitted.
- **Do not rely on running before another subscriber.** Registration order is
  not a contract. A subscriber does run after the generic handler for its event,
  which is what makes a reaction keyed on persisted state safe.

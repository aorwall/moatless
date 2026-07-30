# Loops

A Loop is a standing definition of recurring agent work. Git may declare what a
Loop does, but Git never decides whether it may execute or which user it runs
as. Those are administrator decisions made in the admin UI or with the Moat
CLI.

## What Administrators Can Do

- Explicitly synchronize a repository's committed Loops.
- Activate a Loop by selecting a named run-as user.
- Pause and resume an approved Loop without losing that user.
- Override a Git-synced Loop for local editing.
- Reset an override to the current Git declaration.
- Delete a Git-origin Loop while keeping a restorable tombstone.

Other users may read Loops allowed by their scope, but cannot mutate, activate,
delete, override, reset, or synchronize them.

## How It Works

1. Repository creation and task start may synchronize workspace configuration,
   but never synchronize Loops.
2. An administrator explicitly synchronizes repository configuration to
   reconcile Loops.
3. Git supplies only configuration. Any `active` or run-as values committed in
   a Loop file are ignored and reported as warnings.
4. A new or re-synchronized Git Loop is awaiting approval: it is inactive and
   has no run-as user.
5. Activation selects a valid user and makes the Loop active in one operation.
6. Pausing keeps the selected user; resuming reuses it. Re-synchronizing or
   resetting to Git revokes it.
7. Overriding changes configuration authority to manual while retaining Git
   provenance. Repository sync skips overrides until an administrator resets
   that individual Loop.
8. Deleting a Git-origin Loop keeps a tombstone. Repository sync also skips the
   tombstone; a per-Loop reset restores it.
9. One repository snapshot is reconciled atomically. A malformed present file
   preserves its existing Loop, valid siblings still synchronize, and a
   database failure commits none of the planned Loop changes.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Configuration source | Whether editable Loop configuration is currently owned by Git or manually |
| Execution approval | The administrator decision pairing activation with an explicit run-as user |
| Awaiting approval | Inactive with no selected run-as user |
| Paused | Inactive while retaining a selected run-as user |
| Override | A Git-origin Loop placed under manual configuration control |
| Tombstone | A deleted Git-origin Loop whose provenance is kept for skip and restore behavior |
| Explicit sync | Administrator-initiated reconciliation of repository Loop configuration |

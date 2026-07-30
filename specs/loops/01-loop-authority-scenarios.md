# Loop Configuration and Execution Authority Scenarios

See [00-design.md](./00-design.md) for the design overview.

## Chain 1: Synchronize, approve, and re-synchronize a Git Loop
├── LOOP-01: Explicit sync imports a Loop awaiting approval
│   ├── LOOP-02: Administrator activation selects the execution user
│   │   ├── LOOP-03: Pause and resume preserve the execution user
│   │   └── LOOP-04: Re-sync revokes execution approval
│   └── LOOP-05: Git execution fields are ignored with warnings

## Chain 2: Override and restore Git configuration
├── LOOP-10: A Git Loop is read-only until overridden
│   ├── LOOP-11: Repository sync skips an override
│   └── LOOP-12: Reset to Git discards local configuration and approval

## Chain 3: Delete and restore a Git Loop
├── LOOP-20: Deleting a Git Loop creates a tombstone
└── LOOP-21: Reset to Git restores the tombstone awaiting approval

## Standalone
├── LOOP-30: Non-administrators cannot mutate or synchronize Loops
├── LOOP-31: Automatic repository activity does not synchronize Loops
├── LOOP-32: A malformed present Loop preserves its existing state
└── LOOP-33: Repository reconciliation is atomic

---

### LOOP-01: Explicit sync imports a Loop awaiting approval

Git configuration cannot grant execution authority.

* **Given** a repository whose Git tree declares a valid Loop

* **When** an administrator synchronizes the repository configuration

* **Then** the Loop exists with configuration owned by Git
  * **And** it is awaiting approval
  * **And** it cannot fire

---

### LOOP-02: Administrator activation selects the execution user

Activation and identity selection form one administrator decision.

* **Given** LOOP-01 has completed

* **When** an administrator activates the Loop and selects a user

* **Then** the Loop is active
  * **And** its next eligible firing creates work as the selected user

---

### LOOP-03: Pause and resume preserve the execution user

Routine pausing does not require the administrator to repeat approval.

* **Given** LOOP-02 has completed

* **When** the administrator pauses and then resumes the Loop

* **Then** the Loop becomes active again
  * **And** the previously selected user remains assigned

---

### LOOP-04: Re-sync revokes execution approval

Every accepted Git refresh requires a fresh administrator decision, even when
the committed configuration is unchanged.

* **Given** LOOP-02 has completed

* **When** an administrator synchronizes the repository again

* **Then** the Loop is awaiting approval
  * **And** its selected user has been cleared

---

### LOOP-05: Git execution fields are ignored with warnings

Committed authority fields are visible mistakes, not hidden behavior.

* **Given** the repository declares a Loop as active with a run-as user

* **When** an administrator synchronizes the repository configuration

* **Then** the Loop is still awaiting approval
  * **And** the sync reports both ignored fields as warnings

---

### LOOP-10: A Git Loop is read-only until overridden

The admin UI makes configuration authority explicit.

* **Given** LOOP-01 has completed

* **When** the administrator attempts to edit the Loop directly

* **Then** editing is rejected with guidance to override it
  * **And** overriding it makes the configuration manually owned and editable

---

### LOOP-11: Repository sync skips an override

Local configuration remains stable until that individual Loop is reset.

* **Given** LOOP-10 has completed
  * **And** the administrator has saved a local configuration change

* **When** the administrator synchronizes the repository

* **Then** the local configuration is unchanged
  * **And** the sync reports that it skipped the override

---

### LOOP-12: Reset to Git discards local configuration and approval

Reset is an explicit per-Loop return to Git authority.

* **Given** LOOP-11 has completed

* **When** the administrator resets the Loop to Git

* **Then** its configuration matches the current Git declaration
  * **And** it is awaiting approval

---

### LOOP-20: Deleting a Git Loop creates a tombstone

Deletion survives repository synchronization.

* **Given** LOOP-01 has completed

* **When** an administrator deletes the Loop

* **Then** a deleted manual override retains its Git provenance
  * **And** repository sync does not recreate it

---

### LOOP-21: Reset to Git restores the tombstone awaiting approval

A deleted Git Loop returns only through the explicit per-Loop restore action.

* **Given** LOOP-20 has completed

* **When** the administrator resets the deleted Loop to Git

* **Then** the Loop is visible and Git-owned again
  * **And** it is awaiting approval

---

## Standalone

### LOOP-30: Non-administrators cannot mutate or synchronize Loops

Scope grants read visibility, not Loop administration.

* **Given** a non-administrator can read a Loop

* **When** that user attempts any Loop mutation or repository Loop sync

* **Then** the action is rejected as administrator-only
  * **And** the Loop is unchanged

---

### LOOP-31: Automatic repository activity does not synchronize Loops

Repository creation and task start never introduce or refresh executable
automation.

* **Given** a repository Git tree has a Loop not yet synchronized

* **When** the repository is created or a task starts

* **Then** the Loop is not created or refreshed

---

### LOOP-32: A malformed present Loop preserves its existing state

A broken edit is not a deletion signal.

* **Given** a Git-synced Loop already exists
  * **And** its directory remains present with malformed configuration

* **When** an administrator synchronizes the repository

* **Then** the existing Loop remains unchanged
  * **And** valid sibling Loops are synchronized
  * **And** the malformed Loop is reported as skipped

---

### LOOP-33: Repository reconciliation is atomic

Users never observe a partially applied repository snapshot.

* **Given** a repository sync plans several Loop changes

* **When** persisting any planned Loop change fails

* **Then** none of the planned Loop changes are committed

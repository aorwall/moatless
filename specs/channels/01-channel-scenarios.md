# Channel Scenarios

This spec defines testable scenarios for channel integrations: trigger management, webhook inbound routing with multiple routing modes, outbound webhook callbacks, and error cases. These scenarios use the generic webhook provider to validate the shared channel pipeline.

The E2E suite names these scenarios `INT-nn` rather than `CHAN-nn`, one number to one number — `CHAN-11` is `INT-11`. The suite predates the rename and has not been renumbered.

---

## Overview

### Chain 1: Task Trigger CRUD
```
├── CHAN-01: Create webhook task trigger
│   ├── CHAN-01.1: Trigger appears in listing
│   ├── CHAN-02: Get task trigger by ID
│   ├── CHAN-03: Update task trigger
│   ├── CHAN-04: Deactivate task trigger
│   └── CHAN-05: Delete task trigger
│       └── CHAN-05.1: Trigger removed from listing
```

### Chain 1b: Schedule Loop CRUD
```
├── CHAN-07: Create a schedule loop
│   ├── CHAN-07.1: Schedule loop appears in the schedule listing
│   ├── CHAN-07.2: Updating the cron expression moves the next firing
│   └── CHAN-07.3: Delete the schedule loop
```

### Standalone: Path alias
```
└── CHAN-06: The loops path and its triggers alias answer identically
```

### Standalone: Loop Recent Tasks
```
├── CHAN-45: A schedule loop's tasks are listed, with no binding
└── CHAN-46: A subscription loop's tasks are listed with their binding
```

### Standalone: Task Reply Routing
```
├── CHAN-40: Replying to a task with no binding answers 404
├── CHAN-41: Replying to a task with one binding sends to it
├── CHAN-42: Replying to a task with several bindings answers 400 and lists them
├── CHAN-43: Naming the binding explicitly sends to it
└── CHAN-44: Naming a binding that does not exist answers 404
```

### Chain 2: Webhook Inbound (by_subject routing without thread_id)
```
├── CHAN-10: Create webhook task trigger with by_subject routing
│   ├── CHAN-11: Send message via webhook — task created
│   │   ├── CHAN-11.1: Task is in open state
│   │   └── CHAN-11.2: Task linked to correct repository
│   ├── CHAN-12: Send second message without thread_id — new task created
│   └── CHAN-13: Send message with new thread_id — new task
```

### Chain 3: Webhook Inbound (ongoing routing)
```
├── CHAN-20: Create webhook task trigger with ongoing routing
│   ├── CHAN-21: First message creates ongoing task
│   ├── CHAN-22: Second message routes to same task
│   └── CHAN-23: After task closed, next message creates new task
```

### Chain 4: Webhook Inbound (by_subject routing with thread_id)
```
├── CHAN-30: Create webhook task trigger with by_subject routing
│   ├── CHAN-31: New thread_id creates task
│   ├── CHAN-32: Reply in same thread routes to same task
│   └── CHAN-33: Different thread_id creates new task
├── CHAN-34: A reply after close starts a new task
```

### Chain 5: Webhook Outbound Callback
```
├── CHAN-60: Create webhook task trigger with callback URL
│   ├── CHAN-61: Send inbound message — agent starts
│   ├── CHAN-62: Wait for agent completion
│   └── CHAN-63: Callback server received outbound POST
```

### Standalone: Error Cases
```
├── CHAN-50: Webhook to non-existent trigger returns error
├── CHAN-51: Webhook to inactive trigger is ignored
├── CHAN-52: Webhook with invalid signature is rejected
├── CHAN-53: Create trigger with non-existent repository returns error
└── CHAN-54: Webhook with missing text field returns error
```

---

## Chain 1: Task Trigger CRUD

Task trigger creation, retrieval, updates, deactivation, and deletion.

---

### CHAN-01: Create webhook task trigger

Creating a webhook task trigger for a repository.

* **Given** a valid repository exists

* **When** the user creates a webhook task trigger with a source identifier and by-subject routing

* **Then** the trigger is created in an active state
  * **And** the trigger has a unique identifier
  * **And** the trigger is associated with the correct repository

---

### CHAN-01.1: Trigger appears in listing

A newly created trigger is visible in the listing.

* **Given** CHAN-01 has completed

* **When** the user retrieves the task trigger listing

* **Then** the created trigger appears in the listing

---

### CHAN-02: Get task trigger by ID

Retrieving a task trigger by its identifier.

* **Given** CHAN-01 has completed

* **When** the user retrieves the task trigger by ID

* **Then** the trigger details match what was created
  * **And** the provider is webhook
  * **And** the routing mode is by_subject

---

### CHAN-03: Update task trigger

Updating a task trigger's display name and prompt template.

* **Given** CHAN-01 has completed

* **When** the user updates the trigger's display name and prompt template

* **Then** the updated trigger reflects the new values
  * **And** the routing mode remains unchanged

---

### CHAN-04: Deactivate task trigger

A task trigger can be deactivated without deleting it.

* **Given** CHAN-01 has completed

* **When** the user deactivates the trigger

* **Then** the trigger is inactive
  * **And** the trigger still appears in the listing

---

### CHAN-05: Delete task trigger

Deleting a task trigger removes it from the system.

* **Given** CHAN-01 has completed

* **When** the user deletes the task trigger

* **Then** the operation succeeds

---

### CHAN-05.1: Trigger removed from listing

A deleted trigger no longer appears in the listing.

* **Given** CHAN-05 has completed

* **When** the user retrieves the task trigger listing

* **Then** the deleted trigger is no longer in the listing

---

## Chain 2: Webhook Inbound (by_subject routing without thread_id)

When messages arrive without a thread identifier, each one gets a fresh
synthesized subject and a new task is created. Verifies the create-when-no-binding-exists branch of by_subject routing.

---

### CHAN-10: Create webhook task trigger with by_subject routing

Setting up a webhook task trigger for by_subject routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook task trigger with by_subject routing

* **Then** the trigger is created and active

---

### CHAN-11: Send message via webhook — task created

Sending a message through the webhook creates a new task.

* **Given** CHAN-10 has completed

* **When** a message is sent to the webhook endpoint with text content

* **Then** the webhook accepts the message
  * **And** a new task is created

---

### CHAN-11.1: Task is in open state

The task created by the webhook message is in an open state.

* **Given** CHAN-11 has completed

* **When** retrieving the created task

* **Then** the task is in an open state

---

### CHAN-11.2: Task linked to correct repository

The task created by the webhook is linked to the trigger's repository.

* **Given** CHAN-11 has completed

* **When** retrieving the created task

* **Then** the task is linked to the trigger's repository

---

### CHAN-12: Send second message without thread_id — new task created

A second message without a thread identifier gets its own synthesized subject and creates a separate task.

* **Given** CHAN-11 has completed

* **When** a second message is sent to the same webhook endpoint without a thread identifier

* **Then** a new task is created (different from the first)

---

### CHAN-13: Send message with new thread_id — new task

A message that introduces a new thread identifier has no existing binding, so a new task is created.

* **Given** CHAN-12 has completed

* **When** a message is sent with a thread identifier that has not been used before

* **Then** a new task is created

---

## Chain 3: Webhook Inbound (ongoing routing)

In ongoing mode, all messages route to a single ongoing task until that task is closed.

---

### CHAN-20: Create webhook task trigger with ongoing routing

Setting up a webhook task trigger for ongoing routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook task trigger with ongoing routing

* **Then** the trigger is created and active

---

### CHAN-21: First message creates ongoing task

The first inbound message creates the ongoing task.

* **Given** CHAN-20 has completed

* **When** a message is sent to the webhook endpoint

* **Then** the webhook accepts the message
  * **And** a task is created and linked to the trigger as the ongoing task

---

### CHAN-22: Second message routes to same task

Subsequent messages route to the existing ongoing task.

* **Given** CHAN-21 has completed

* **When** a second message is sent to the same webhook endpoint

* **Then** no new task is created
  * **And** the trigger still points to the same ongoing task

---

### CHAN-23: After task closed, next message creates new task

When the ongoing task is closed, the next message creates a new task.

* **Given** CHAN-22 has completed and the ongoing task has been closed

* **When** a message is sent to the webhook endpoint

* **Then** a new ongoing task is created
  * **And** the trigger points to the new task

---

## Chain 4: Webhook Inbound (by_subject routing with thread_id)

When messages arrive with a thread identifier, the thread becomes the subject and messages on the same thread reuse the task. Verifies the find-existing-binding branch of by_subject routing.

---

### CHAN-30: Create webhook task trigger with by_subject routing

Setting up a webhook task trigger for by_subject routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook task trigger with by_subject routing

* **Then** the trigger is created and active

---

### CHAN-31: New thread_id creates task

A message with a never-seen thread identifier creates a new task.

* **Given** CHAN-30 has completed

* **When** a message is sent with a thread identifier

* **Then** a new task is created

---

### CHAN-32: Reply in same thread routes to same task

A reply in the same thread routes to the existing task.

* **Given** CHAN-31 has completed

* **When** a message is sent with the same thread identifier

* **Then** no new task is created
  * **And** the message routes to the existing task

---

### CHAN-33: Different thread_id creates new task

A message with a different thread identifier creates a separate task.

* **Given** CHAN-32 has completed

* **When** a message is sent with a different thread identifier

* **Then** a new task is created (separate from the first thread's task)

---

## Chain 5: Webhook Outbound Callback

When a webhook task trigger has a callback URL configured, the backend delivers outbound results to that URL when the agent completes work.

---

### CHAN-60: Create webhook task trigger with callback URL

Setting up a webhook task trigger with an outbound callback URL.

* **Given** a valid repository exists

* **When** the user creates a webhook task trigger with a callback URL

* **Then** the trigger is created and active

---

### CHAN-61: Send inbound message — agent starts

Sending a message via the webhook triggers agent execution.

* **Given** CHAN-60 has completed

* **When** a message is sent to the webhook endpoint

* **Then** the webhook accepts the message
  * **And** a task is created and the agent begins processing

---

### CHAN-62: Wait for agent completion

The agent completes processing the message.

* **Given** CHAN-61 has completed

* **When** waiting for the agent to finish its turn

* **Then** the agent completes successfully

---

### CHAN-63: Callback server received outbound POST

The callback URL receives the agent's result.

* **Given** CHAN-62 has completed

* **When** checking the callback server for received requests

* **Then** the callback server received a POST with the task result
  * **And** the result includes the task identifier and response text
  * **And** the result indicates success (no error)

---

## Standalone: Error Cases

These scenarios are independent and test specific error conditions.

---

### CHAN-50: Webhook to non-existent trigger returns error

* **Given** no task trigger exists with the given ID

* **When** a message is sent to the webhook endpoint for the non-existent trigger

* **Then** the system returns an error indicating the trigger was not found

---

### CHAN-51: Webhook to inactive trigger is ignored

* **Given** a webhook task trigger exists but has been deactivated

* **When** a message is sent to the webhook endpoint for the inactive trigger

* **Then** the system accepts the message but does not create a task

---

### CHAN-52: Webhook with invalid signature is rejected

* **Given** a webhook task trigger exists with a signing secret configured

* **When** a message is sent with an invalid signature

* **Then** the system rejects the message with an authentication error

---

### CHAN-53: Create trigger with non-existent repository returns error

* **Given** no repository exists with the given ID

* **When** the user creates a task trigger referencing the non-existent repository

* **Then** the system returns an error indicating the repository was not found

---

### CHAN-54: Webhook with missing text field returns error

* **Given** a webhook task trigger exists

* **When** a message is sent without a text field

* **Then** the system returns a validation error

---

## Chain 1b: Schedule Loop CRUD

A loop fired by a cron expression it carries, rather than by an inbound adapter
event. Same resource, same endpoint, different source.

---

### CHAN-07: Create a schedule loop

A schedule loop is stored with its cron expression, timezone and message
template, and the server computes when it will next fire.

* **Given** a repository exists

* **When** the user creates a schedule loop with a cron expression and a message template

* **Then** the loop is of kind schedule, carrying the repository, task name and prompt it was given
  * **And** its source carries the cron expression, timezone and message template unchanged
  * **And** it has never fired
  * **And** its next firing is in the future

---

### CHAN-07.1: Schedule loop appears in the schedule listing

* **Given** CHAN-07 has completed

* **When** the user lists loops of kind schedule

* **Then** the created loop appears

---

### CHAN-07.2: Updating the cron expression moves the next firing

The next firing is derived, not stored input — changing the expression
recomputes it.

* **Given** CHAN-07 has completed

* **When** the user updates the loop's cron expression, timezone and message template

* **Then** the source carries the new values
  * **And** the next firing is recomputed to a time after the update

---

### CHAN-07.3: Delete the schedule loop

* **Given** CHAN-07 has completed

* **When** the user deletes the loop

* **Then** it no longer appears in the schedule listing

---

## Standalone: Path alias

---

### CHAN-06: The loops path and its triggers alias answer identically

`/api/v1/loops` is the documented path; `/api/v1/triggers` is the older name
kept working for existing clients. They are the same handlers, so the same
operation must answer identically on either.

* **Given** a subscription loop exists

* **When** the loop is read by id through both paths

* **Then** the two responses are identical

* **When** loops are listed through both paths with the same filters

* **Then** the created loop appears in both
  * **And** the two listings are identical

---

## Standalone: Task Reply Routing

A reply has to go somewhere, and where is decided by the task's bindings — a
binding records which external subject (a webhook thread, a pull request, a
Slack thread) a task answers into. These scenarios are independent.

---

### CHAN-40: Replying to a task with no binding answers 404

* **Given** a task with no binding of any kind

* **When** the user replies to the task

* **Then** the system answers 404
  * **And** the error says the task has no bindings

---

### CHAN-41: Replying to a task with one binding sends to it

An inbound message creates both the task and the binding it answers into, so
one destination needs no choosing.

* **Given** a webhook subscription loop exists
  * **And** an inbound message on it created a task

* **When** the user replies to that task without naming a binding

* **Then** exactly one destination is sent to
  * **And** no provider error is reported
  * **And** the reported target is the binding

---

### CHAN-42: Replying to a task with several bindings answers 400 and lists them

With more than one destination the system refuses to guess, and the refusal
carries what a caller needs to choose.

* **Given** a task carrying two bindings

* **When** the user replies to the task without naming one

* **Then** the system answers 400
  * **And** both candidate targets are listed
  * **And** each carries the flag that would select it

---

### CHAN-43: Naming the binding explicitly sends to it

* **Given** a task carrying a binding

* **When** the user replies naming that binding

* **Then** exactly one destination is sent to

---

### CHAN-44: Naming a binding that does not exist answers 404

* **Given** a task exists

* **When** the user replies naming a binding id that does not exist

* **Then** the system answers 404

---

## Standalone: Loop Recent Tasks

The tasks a loop produced are listed from the task's own loop id, not from a
binding. A schedule loop never creates a binding, so a listing driven by
bindings would report that a loop firing every night has produced nothing.

---

### CHAN-45: A schedule loop's tasks are listed, with no binding

* **Given** a schedule loop with a task attributed to it

* **When** the user lists the loop's recent tasks

* **Then** the task is listed
  * **And** it carries no binding

---

### CHAN-46: A subscription loop's tasks are listed with their binding

* **Given** a webhook subscription loop
  * **And** an inbound message on it created a task

* **When** the user lists the loop's recent tasks

* **Then** the created task is listed
  * **And** it carries the binding the inbound message created

---

## Standalone: The GitHub Proxy

The platform posts to GitHub on a task's behalf rather than letting the caller
do it directly. That is what lets the message carry the task's identity, bind
the task to what it replied to, and be recognised later as the platform's own
comment rather than a new inbound event.

---

### CHAN-70: Opening a pull request for a task that does not exist is refused

Access to the task is settled before anything is sent to GitHub, so a bad task reference never reaches the outside world.

* **Given** no task with the given identifier

* **When** the user opens a pull request on that task's behalf

* **Then** the system reports that the task was not found

---

### CHAN-71: Commenting for a task that does not exist is refused

* **Given** no task with the given identifier

* **When** the user comments on that task's behalf

* **Then** the system reports that the task was not found

---

### CHAN-72: Replying for a task that does not exist is refused

* **Given** no task with the given identifier

* **When** the user replies to a review comment on that task's behalf

* **Then** the system reports that the task was not found

---

### CHAN-73: Opening a pull request with no usable GitHub credentials fails

Having no credentials and having credentials GitHub rejects are the same situation from the user's side: the request could not be completed with GitHub.

* **Given** the platform has no usable credentials for the target repository

* **When** the user opens a pull request

* **Then** the system reports that the request could not be completed upstream

---

### CHAN-74: A pull request, a comment and a reply all bind the one task

Everything the platform sends about one pull request belongs to one conversation, so a later reply on any of it comes back to the same task.

* **Given** a task, and a repository the platform holds credentials for

* **When** the user opens a pull request for the task without naming a base branch
  * **And** comments on it
  * **And** replies to a review comment on it

* **Then** the pull request is opened against the repository's default branch
  * **And** its description keeps what the user wrote and gains a link back to the task
  * **And** the task is bound to that pull request
  * **And** the comment and the reply leave it bound to the same one, exactly once

---

### CHAN-34: A reply after close starts a new task

Closing a task is final. A later reply in the same thread starts a new one — routing it back would reopen work the user has finished with, and would leave a message on a task nobody is watching.

* **Given** a webhook task trigger routing by subject
  * **And** an inbound message in a thread created a task
  * **And** the user closed that task

* **When** another message arrives in the same thread

* **Then** a new task is created for it
  * **And** the closed task stays closed
  * **And** nothing is added to the closed task's conversation

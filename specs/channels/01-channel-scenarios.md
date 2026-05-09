# Channel Scenarios

This spec defines testable scenarios for channel integrations: trigger management, webhook inbound routing with multiple routing modes, outbound webhook callbacks, and error cases. These scenarios use the generic webhook provider to validate the shared channel pipeline.

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

### Chain 2: Webhook Inbound (per_message routing)
```
├── CHAN-10: Create webhook task trigger with per_message routing
│   ├── CHAN-11: Send message via webhook — task created
│   │   ├── CHAN-11.1: Task is in open state
│   │   └── CHAN-11.2: Task linked to correct repository
│   ├── CHAN-12: Send second message — new task created
│   └── CHAN-13: Send message with thread_id — new task
```

### Chain 3: Webhook Inbound (ongoing routing)
```
├── CHAN-20: Create webhook task trigger with ongoing routing
│   ├── CHAN-21: First message creates ongoing task
│   ├── CHAN-22: Second message routes to same task
│   └── CHAN-23: After task closed, next message creates new task
```

### Chain 4: Webhook Inbound (thread_based routing)
```
├── CHAN-30: Create webhook task trigger with thread_based routing
│   ├── CHAN-31: New message creates task
│   ├── CHAN-32: Reply in same thread routes to same task
│   └── CHAN-33: New thread creates new task
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

* **When** the user creates a webhook task trigger with a source identifier and per-message routing

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
  * **And** the routing mode is per_message

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

## Chain 2: Webhook Inbound (per_message routing)

Each inbound webhook message creates a new task.

---

### CHAN-10: Create webhook task trigger with per_message routing

Setting up a webhook task trigger for per-message routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook task trigger with per_message routing

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

### CHAN-12: Send second message — new task created

In per_message mode, a second message creates a separate task.

* **Given** CHAN-11 has completed

* **When** a second message is sent to the same webhook endpoint

* **Then** a new task is created (different from the first)

---

### CHAN-13: Send message with thread_id — new task

In per_message mode, a message with a thread identifier still creates a new task.

* **Given** CHAN-12 has completed

* **When** a message is sent with a thread identifier

* **Then** a new task is created (per_message mode ignores thread identifiers)

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

## Chain 4: Webhook Inbound (thread_based routing)

In thread_based mode, messages are grouped by thread identifier — messages in the same thread route to the same task.

---

### CHAN-30: Create webhook task trigger with thread_based routing

Setting up a webhook task trigger for thread-based routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook task trigger with thread_based routing

* **Then** the trigger is created and active

---

### CHAN-31: New message creates task

A message with a new thread identifier creates a new task.

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

### CHAN-33: New thread creates new task

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

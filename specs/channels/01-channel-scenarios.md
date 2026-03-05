# Channel Scenarios

This spec defines testable scenarios for channel integrations: config management, webhook inbound routing with multiple routing modes, cross-channel response routing, outbound webhook callbacks, and error cases. These scenarios use the generic webhook provider to validate the shared channel pipeline.

---

## Overview

### Chain 1: Channel Config CRUD
```
├── CHAN-01: Create webhook channel config
│   ├── CHAN-01.1: Config appears in listing
│   ├── CHAN-02: Get channel config by ID
│   ├── CHAN-03: Update channel config
│   ├── CHAN-04: Deactivate channel config
│   └── CHAN-05: Delete channel config
│       └── CHAN-05.1: Config removed from listing
```

### Chain 2: Webhook Inbound (per_message routing)
```
├── CHAN-10: Create webhook config with per_message routing
│   ├── CHAN-11: Send message via webhook — task created
│   │   ├── CHAN-11.1: Task is in open state
│   │   └── CHAN-11.2: Task linked to correct repository
│   ├── CHAN-12: Send second message — new task created
│   └── CHAN-13: Send message with thread_id — new task
```

### Chain 3: Webhook Inbound (ongoing routing)
```
├── CHAN-20: Create webhook config with ongoing routing
│   ├── CHAN-21: First message creates ongoing task
│   ├── CHAN-22: Second message routes to same task
│   └── CHAN-23: After task closed, next message creates new task
```

### Chain 4: Webhook Inbound (thread_based routing)
```
├── CHAN-30: Create webhook config with thread_based routing
│   ├── CHAN-31: New message creates task
│   ├── CHAN-32: Reply in same thread routes to same task
│   └── CHAN-33: New thread creates new task
```

### Chain 5: Cross-Channel Response Routing
```
├── CHAN-40: Create two webhook configs (trigger + response)
│   ├── CHAN-41: Link trigger config to response config
│   └── CHAN-42: Inbound on trigger creates task with cross-channel routing
```

### Chain 6: Webhook Outbound Callback
```
├── CHAN-60: Create webhook config with callback URL
│   ├── CHAN-61: Send inbound message — agent starts
│   ├── CHAN-62: Wait for agent completion
│   └── CHAN-63: Callback server received outbound POST
```

### Standalone: Error Cases
```
├── CHAN-50: Webhook to non-existent config returns error
├── CHAN-51: Webhook to inactive config is ignored
├── CHAN-52: Webhook with invalid signature is rejected
├── CHAN-53: Create config with non-existent repository returns error
└── CHAN-54: Webhook with missing text field returns error
```

---

## Chain 1: Channel Config CRUD

Channel config creation, retrieval, updates, deactivation, and deletion.

---

### CHAN-01: Create webhook channel config

Creating a webhook channel config for a repository.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with a source identifier and per-message routing

* **Then** the config is created in an active state
  * **And** the config has a unique identifier
  * **And** the config is associated with the correct repository

---

### CHAN-01.1: Config appears in listing

A newly created config is visible in the listing.

* **Given** CHAN-01 has completed

* **When** the user retrieves the channel config listing

* **Then** the created config appears in the listing

---

### CHAN-02: Get channel config by ID

Retrieving a channel config by its identifier.

* **Given** CHAN-01 has completed

* **When** the user retrieves the channel config by ID

* **Then** the config details match what was created
  * **And** the provider is webhook
  * **And** the routing mode is per_message

---

### CHAN-03: Update channel config

Updating a channel config's display name and prompt template.

* **Given** CHAN-01 has completed

* **When** the user updates the config's display name and prompt template

* **Then** the updated config reflects the new values
  * **And** the routing mode remains unchanged

---

### CHAN-04: Deactivate channel config

A channel config can be deactivated without deleting it.

* **Given** CHAN-01 has completed

* **When** the user deactivates the config

* **Then** the config is inactive
  * **And** the config still appears in the listing

---

### CHAN-05: Delete channel config

Deleting a channel config removes it from the system.

* **Given** CHAN-01 has completed

* **When** the user deletes the channel config

* **Then** the operation succeeds

---

### CHAN-05.1: Config removed from listing

A deleted config no longer appears in the listing.

* **Given** CHAN-05 has completed

* **When** the user retrieves the channel config listing

* **Then** the deleted config is no longer in the listing

---

## Chain 2: Webhook Inbound (per_message routing)

Each inbound webhook message creates a new task.

---

### CHAN-10: Create webhook config with per_message routing

Setting up a webhook config for per-message routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with per_message routing

* **Then** the config is created and active

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

The task created by the webhook is linked to the config's repository.

* **Given** CHAN-11 has completed

* **When** retrieving the created task

* **Then** the task is linked to the config's repository

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

### CHAN-20: Create webhook config with ongoing routing

Setting up a webhook config for ongoing routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with ongoing routing

* **Then** the config is created and active

---

### CHAN-21: First message creates ongoing task

The first inbound message creates the ongoing task.

* **Given** CHAN-20 has completed

* **When** a message is sent to the webhook endpoint

* **Then** the webhook accepts the message
  * **And** a task is created and linked to the config as the ongoing task

---

### CHAN-22: Second message routes to same task

Subsequent messages route to the existing ongoing task.

* **Given** CHAN-21 has completed

* **When** a second message is sent to the same webhook endpoint

* **Then** no new task is created
  * **And** the config still points to the same ongoing task

---

### CHAN-23: After task closed, next message creates new task

When the ongoing task is closed, the next message creates a new task.

* **Given** CHAN-22 has completed and the ongoing task has been closed

* **When** a message is sent to the webhook endpoint

* **Then** a new ongoing task is created
  * **And** the config points to the new task

---

## Chain 4: Webhook Inbound (thread_based routing)

In thread_based mode, messages are grouped by thread identifier — messages in the same thread route to the same task.

---

### CHAN-30: Create webhook config with thread_based routing

Setting up a webhook config for thread-based routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with thread_based routing

* **Then** the config is created and active

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

## Chain 5: Cross-Channel Response Routing

A trigger config can route outbound replies to a different response config's provider.

---

### CHAN-40: Create two webhook configs (trigger + response)

Setting up two webhook configs for cross-channel routing.

* **Given** a valid repository exists

* **When** the user creates two webhook channel configs (one trigger, one response)

* **Then** both configs are created and active

---

### CHAN-41: Link trigger config to response config

Linking the trigger config to the response config for cross-channel routing.

* **Given** CHAN-40 has completed

* **When** the user updates the trigger config with the response config as its response destination

* **Then** the trigger config shows the response destination link

---

### CHAN-42: Inbound on trigger creates task with cross-channel routing

Sending a message to the trigger config creates a task with the cross-channel link intact.

* **Given** CHAN-41 has completed

* **When** a message is sent to the trigger config's webhook endpoint

* **Then** a new task is created
  * **And** the trigger config still has the response destination linked

---

## Chain 6: Webhook Outbound Callback

When a webhook config has a callback URL configured, the backend delivers outbound results to that URL when the agent completes work.

---

### CHAN-60: Create webhook config with callback URL

Setting up a webhook config with an outbound callback URL.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with a callback URL

* **Then** the config is created and active

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

### CHAN-50: Webhook to non-existent config returns error

* **Given** no channel config exists with the given ID

* **When** a message is sent to the webhook endpoint for the non-existent config

* **Then** the system returns an error indicating the config was not found

---

### CHAN-51: Webhook to inactive config is ignored

* **Given** a webhook channel config exists but has been deactivated

* **When** a message is sent to the webhook endpoint for the inactive config

* **Then** the system accepts the message but does not create a task

---

### CHAN-52: Webhook with invalid signature is rejected

* **Given** a webhook channel config exists with a signing secret configured

* **When** a message is sent with an invalid signature

* **Then** the system rejects the message with an authentication error

---

### CHAN-53: Create config with non-existent repository returns error

* **Given** no repository exists with the given ID

* **When** the user creates a channel config referencing the non-existent repository

* **Then** the system returns an error indicating the repository was not found

---

### CHAN-54: Webhook with missing text field returns error

* **Given** a webhook channel config exists

* **When** a message is sent without a text field

* **Then** the system returns a validation error

# Channels

This spec defines testable scenarios for channel integrations: config management, webhook inbound routing, and cross-channel response routing. These scenarios use the generic webhook provider to validate the shared channel pipeline.

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
│   │   ├── CHAN-11.2: Task linked to correct repository
│   │   └── CHAN-12: Send second message — new task created
│   └── CHAN-13: Send message with thread_id — no existing thread, new task
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
│   ├── CHAN-41: Link trigger config to response config via response_config_id
│   └── CHAN-42: Inbound on trigger config creates task; outbound goes to response config callback
```

### Chain 6: Webhook Outbound Callback (requires sandbox)
```
├── CHAN-60: Create webhook config with callback URL pointing to test server
├── CHAN-61: Send inbound webhook message (creates task + starts agent)
├── CHAN-62: Wait for agent completion
└── CHAN-63: Verify callback server received outbound POST
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

Channel config creation, retrieval, updates, and deletion. This chain tests config management without requiring webhook message delivery.

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

A newly created config is visible in the config listing.

* **Given** CHAN-01 has completed

* **When** the user lists all channel configs

* **Then** the response includes the created config
  * **And** the config is in an active state

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

Updating a channel config's properties.

* **Given** CHAN-01 has completed

* **When** the user updates the config's display name and prompt template

* **Then** the updated config reflects the new values
  * **And** the routing mode remains unchanged

---

### CHAN-04: Deactivate channel config

Deactivating a channel config.

* **Given** CHAN-01 has completed

* **When** the user sets the config's active flag to false

* **Then** the config is inactive
  * **And** the config still exists in the listing

---

### CHAN-05: Delete channel config

Deleting a channel config.

* **Given** CHAN-01 has completed

* **When** the user deletes the channel config

* **Then** the operation succeeds

---

### CHAN-05.1: Config removed from listing

A deleted config is no longer visible.

* **Given** CHAN-05 has completed

* **When** the user lists all channel configs

* **Then** the deleted config is not in the listing

---

## Chain 2: Webhook Inbound (per_message routing)

Each inbound webhook message creates a new task. Thread replies continue existing tasks.

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

---

### CHAN-11.1: Task is in open state

The task created by the webhook message is in an open state.

* **Given** CHAN-11 has completed

* **When** the most recent task for the repository is retrieved

* **Then** the task is in an open state

---

### CHAN-11.2: Task linked to correct repository

The task created by the webhook is associated with the correct repository.

* **Given** CHAN-11 has completed

* **When** the most recent task for the repository is retrieved

* **Then** the task's repository matches the config's repository

---

### CHAN-12: Send second message — new task created

In per_message mode, a second message creates a separate task.

* **Given** CHAN-11 has completed

* **When** a second message is sent to the same webhook endpoint

* **Then** a new task is created (different from the first)

---

### CHAN-13: Send message with thread_id — no existing thread, new task

A message with a thread_id that doesn't match an existing thread creates a new task.

* **Given** CHAN-10 has completed

* **When** a message with a new thread_id is sent to the webhook endpoint

* **Then** a new task is created

---

## Chain 3: Webhook Inbound (ongoing routing)

All messages route to a single ongoing task. When the task is closed, the next message creates a new one.

---

### CHAN-20: Create webhook config with ongoing routing

Setting up a webhook config for ongoing routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with ongoing routing

* **Then** the config is created and active

---

### CHAN-21: First message creates ongoing task

The first message creates a new ongoing task.

* **Given** CHAN-20 has completed

* **When** a message is sent to the webhook endpoint

* **Then** a task is created in an open state
  * **And** the config's ongoing task is set to the new task

---

### CHAN-22: Second message routes to same task

A second message routes to the same ongoing task.

* **Given** CHAN-21 has completed

* **When** a second message is sent to the same webhook endpoint

* **Then** the message routes to the existing task (no new task created)

---

### CHAN-23: After task closed, next message creates new task

After the ongoing task is closed, the next message creates a new one.

* **Given** CHAN-22 has completed
  * **And** the ongoing task has been closed

* **When** a new message is sent to the webhook endpoint

* **Then** a new task is created
  * **And** the config's ongoing task is updated to the new task

---

## Chain 4: Webhook Inbound (thread_based routing)

New messages create tasks; replies within the same thread continue the existing task.

---

### CHAN-30: Create webhook config with thread_based routing

Setting up a webhook config for thread-based routing mode.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with thread_based routing

* **Then** the config is created and active

---

### CHAN-31: New message creates task

A new message without a thread_id creates a task.

* **Given** CHAN-30 has completed

* **When** a message without a thread_id is sent to the webhook endpoint

* **Then** a new task is created

---

### CHAN-32: Reply in same thread routes to same task

A reply in the same thread routes to the existing task.

* **Given** CHAN-31 has completed

* **When** a message with the same thread_id is sent to the webhook endpoint

* **Then** the message routes to the existing task

---

### CHAN-33: New thread creates new task

A message with a different thread_id creates a new task.

* **Given** CHAN-31 has completed

* **When** a message with a new thread_id is sent to the webhook endpoint

* **Then** a new task is created (different from the first)

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

* **When** the user updates the trigger config with the response config's ID as response_config_id

* **Then** the trigger config shows the response_config_id link

---

### CHAN-42: Inbound on trigger creates task with cross-channel routing configured

Verifying that a task created via the trigger config will have cross-channel routing active.

* **Given** CHAN-41 has completed

* **When** a message is sent to the trigger webhook endpoint

* **Then** a task is created
  * **And** the originating config has a response_config_id pointing to the response config

---

## Chain 6: Webhook Outbound Callback

Verifies that the backend delivers outbound callback POSTs to the webhook config's callback URL when an agent completes. Requires a running sandbox environment.

---

### CHAN-60: Create webhook config with callback URL pointing to test server

Setting up a webhook config whose outbound replies will be sent to a local test server.

* **Given** a valid repository exists
  * **And** a local HTTP server is listening for callback POSTs

* **When** the user creates a webhook channel config with a callback_url pointing to the local server

* **Then** the config is created and active

---

### CHAN-61: Send inbound webhook message (creates task + starts agent)

An inbound message triggers task creation and agent execution.

* **Given** CHAN-60 has completed

* **When** a message is sent to the webhook endpoint

* **Then** a new task is created
  * **And** the agent begins execution

---

### CHAN-62: Wait for agent completion

The agent completes execution on the task.

* **Given** CHAN-61 has completed

* **When** the agent finishes processing the message

* **Then** the agent transitions to an idle state

---

### CHAN-63: Verify callback server received outbound POST

The backend sends an outbound reply to the webhook config's callback URL.

* **Given** CHAN-62 has completed

* **When** the outbound notification is processed

* **Then** the local test server receives an HTTP POST
  * **And** the body contains the task identifier and reply text
  * **And** the body indicates no error occurred
  * **And** the content type is JSON

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

* **Given** a webhook channel config exists but is inactive

* **When** a message is sent to the webhook endpoint

* **Then** the system accepts the request but does not create a task

---

### CHAN-52: Webhook with invalid signature is rejected

* **Given** a webhook channel config exists with a webhook_secret configured

* **When** a message is sent with an invalid signature

* **Then** the system returns an authentication error

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

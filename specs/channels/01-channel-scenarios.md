# Channel Scenarios

This spec defines testable scenarios for channel integrations: config management, webhook inbound routing, and cross-channel response routing. These scenarios use the generic webhook provider to validate the shared channel pipeline.

---

## Overview

### Chain 1: Channel Config CRUD
```
├── CHAN-01: Create webhook channel config
│   ├── CHAN-02: Get channel config by ID
│   ├── CHAN-03: Update channel config
│   └── CHAN-05: Delete channel config
```

### Chain 2: Webhook Inbound (per_message routing)
```
├── CHAN-10: Create webhook config with per_message routing
│   ├── CHAN-11: Send message via webhook — task created
│   └── CHAN-12: Send second message — new task created
```

### Chain 3: Cross-Channel Response Routing
```
├── CHAN-40: Create two webhook configs (trigger + response)
│   └── CHAN-41: Link trigger config to response config
```

### Standalone: Error Cases
```
├── CHAN-50: Webhook to non-existent config returns error
├── CHAN-53: Create config with non-existent repository returns error
└── CHAN-54: Webhook with missing text field returns error
```

---

## Chain 1: Channel Config CRUD

Channel config creation, retrieval, updates, and deletion.

---

### CHAN-01: Create webhook channel config

Creating a webhook channel config for a repository.

* **Given** a valid repository exists

* **When** the user creates a webhook channel config with a source identifier and per-message routing

* **Then** the config is created in an active state
  * **And** the config has a unique identifier
  * **And** the config is associated with the correct repository
  * **And** the config appears in the channel config listing

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

Updating a channel config's properties, including deactivation.

* **Given** CHAN-01 has completed

* **When** the user updates the config's display name, prompt template, and sets active to false

* **Then** the updated config reflects the new values
  * **And** the routing mode remains unchanged
  * **And** the config is inactive

---

### CHAN-05: Delete channel config

Deleting a channel config removes it from the system.

* **Given** CHAN-01 has completed

* **When** the user deletes the channel config

* **Then** the operation succeeds
  * **And** the deleted config is no longer in the listing

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
  * **And** a new task is created in an open state
  * **And** the task is linked to the config's repository

---

### CHAN-12: Send second message — new task created

In per_message mode, a second message creates a separate task.

* **Given** CHAN-11 has completed

* **When** a second message is sent to the same webhook endpoint

* **Then** a new task is created (different from the first)

---

## Chain 3: Cross-Channel Response Routing

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

## Standalone: Error Cases

These scenarios are independent and test specific error conditions.

---

### CHAN-50: Webhook to non-existent config returns error

* **Given** no channel config exists with the given ID

* **When** a message is sent to the webhook endpoint for the non-existent config

* **Then** the system returns an error indicating the config was not found

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

# Agent Execution Scenarios

This spec defines all testable scenarios for agent execution: happy-path execution and multi-turn conversation, stopping an agent mid-execution, and resuming after a sandbox restart.

For related scenarios, see:
- [Task scenarios](../tasks/01-task-scenarios.md) — task management
- [Sandbox scenarios](../sandbox/01-sandbox-scenarios.md) — sandbox provisioning and lifecycle
- [File scenarios](../sandbox/02-file-scenarios.md) — standalone file operations

---

## Overview

### Chain 1: Agent Execution & Multi-Turn

```
├── AGENT-01: Agent executes initial message
│   ├── AGENT-01.3: Conversation history available
│   ├── AGENT-01.4: Task cost increases
│   ├── FILE-06: Files reflect agent changes
│   └── AGENT-02: Multi-turn conversation
│       └── AGENT-02.3: Files reflect second-turn changes
```

### Chain 2: Agent Stop & Resume

```
├── AGENT-04: Stop agent mid-execution
└── AGENT-06: Resume after sandbox restart
```

---

## Chain 1: Agent Execution & Multi-Turn

The happy-path flow from task creation through multi-turn agent conversation. This chain exercises the agent execution experience: create a task, let the agent work, verify files, then have a follow-up conversation.

Each scenario below depends on its parent as a precondition. A task with an initial message is created first, and the sandbox provisions automatically.

---

### AGENT-01: Agent executes initial message

The agent processes the task's initial message, performing code changes in the workspace.

* **Given** a task has been created with an initial message and the sandbox is ready

* **When** waiting for the agent to complete its first turn

* **Then** the agent has completed its first turn
  * **And** the agent is idle
  * **And** the agent produced at least one message during execution

---

### AGENT-01.3: Conversation history available

The conversation history reflects the messages after agent execution.

* **Given** AGENT-01 has completed

* **When** retrieving the task's messages

* **Then** the response contains at least the initial user message
  * **And** the response contains at least one assistant message

---

### AGENT-01.4: Task cost increases

Agent execution consumes API tokens, reflected in the task's cost tracking.

* **Given** AGENT-01 has completed

* **When** retrieving the task details

* **Then** the total cost is greater than zero

---

### FILE-06: Files reflect agent changes

The workspace files show changes made by the agent during execution.

* **Given** AGENT-01 has completed

* **When** searching workspace files for expected content (e.g., the text requested in the initial message)

* **Then** at least one file contains the expected content
  * **And** the git diff shows changed files

---

### AGENT-02: Multi-turn conversation

A follow-up message triggers a second agent turn, demonstrating session continuity.

* **Given** AGENT-01 has completed (first turn done, agent is idle)

* **When** the user sends a follow-up message

* **Then** the agent activates for a second turn
  * **And** the agent completes the second turn and returns to idle

---

### AGENT-02.3: Files reflect second-turn changes

The workspace reflects modifications from the second agent turn.

* **Given** AGENT-02 has completed

* **When** searching workspace files for the content requested in the follow-up message

* **Then** at least one file contains the updated content

---

## Chain 2: Agent Stop & Resume

Stopping an agent mid-execution and resuming after a sandbox restart.

Each scenario below depends on its parent as a precondition. A task with an initial message is created first, and the agent starts executing.

---

### AGENT-04: Stop agent mid-execution

A running agent can be stopped by the user before it completes.

* **Given** a task has been created and the agent is running

* **When** the user stops the task

* **Then** the agent stops executing

---

### AGENT-06: Resume after sandbox restart

After stopping, the sandbox can be restarted and the agent resumes with full conversation continuity.

* **Given** AGENT-04 has completed (agent stopped)

* **When** the sandbox is stopped, restarted, and a follow-up message is sent

* **Then** the agent completes a new turn
  * **And** the conversation history includes messages from before and after the restart
  * **And** there are at least two user messages in the history

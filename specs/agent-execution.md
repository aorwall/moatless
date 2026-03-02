# Agent Execution

This spec defines all testable scenarios for agent execution: happy-path execution and multi-turn conversation, plan mode (agent plans before executing), stopping an agent mid-execution, resuming after a sandbox restart, and error recovery.

For the basic happy-path flow (task creation, sandbox provisioning, preview server), see:
- [Task CRUD](task-crud.md) — task management
- [Sandbox Operations](sandbox-operations.md) — sandbox provisioning and lifecycle

---

## Overview

### Chain 1: Agent Execution & Multi-Turn

```
├── TASK-01: Create task with message
│   └── SANDBOX-01: Sandbox provisions automatically
│       └── AGENT-01: Agent executes initial message
│           ├── AGENT-01.1: Agent state transitions
│           ├── AGENT-01.2: Agent lifecycle notifications
│           ├── AGENT-01.3: Conversation history available
│           ├── AGENT-01.4: Task cost increases
│           ├── FILE-06: Files reflect agent changes
│           └── AGENT-02: Multi-turn conversation
│               ├── AGENT-02.1: Agent re-activates on follow-up
│               ├── AGENT-02.2: Second turn-completed notification
│               └── AGENT-02.3: Files reflect second-turn changes
```

### Chain 2: Plan Mode

```
├── TASK-01: Create task in plan mode
│   └── SANDBOX-01: Sandbox provisions
│       └── AGENT-03: Plan mode flow
│           ├── AGENT-03.1: Agent produces plan for approval
│           └── AGENT-03.2: User approves, agent executes
```

### Chain 3: Agent Stop, Resume & Recovery

```
├── TASK-01: Create task with message (agent starts)
│   ├── AGENT-04: Stop agent mid-execution
│   │   ├── AGENT-04.1: Sandbox remains ready
│   │   └── AGENT-04.2: Agent-stopped notifications
│   ├── AGENT-06: Resume after sandbox restart
│   │   ├── AGENT-06.1: Sandbox stops
│   │   ├── AGENT-06.2: Sandbox restarts with files intact
│   │   ├── AGENT-06.3: Agent resumes with follow-up message
│   │   ├── AGENT-06.4: Agent retains conversation history
│   │   └── AGENT-06.5: Files reflect resumed-session changes
│   └── AGENT-05: Agent error sets task to error (skipped)
│       ├── AGENT-05.1: Error visible in chat (skipped)
│       └── AGENT-05.2: Recovery via new message (skipped)
```

---

## Chain 1: Agent Execution & Multi-Turn

The happy-path flow from task creation through multi-turn agent conversation. This chain exercises the agent execution experience: create a task, let the agent work, verify files, then have a follow-up conversation.

Each scenario below depends on its parent as a precondition.

---

### TASK-01: Create task with message

Creating a task with an initial message triggers sandbox provisioning and agent execution.

* **Given** a valid repository exists (a template repo has been registered)

* **When** the user creates a task with a repository and an initial message

* **Then** the task is created in an open state
  * **And** the task has a unique identifier
  * **And** the initial message is included in the task

---

### SANDBOX-01: Sandbox provisions automatically

The sandbox is automatically provisioned when a task is created.

* **Given** TASK-01 has completed (task exists in open state)

* **When** checking the sandbox status

* **Then** the sandbox becomes ready
  * **And** the sandbox is healthy

---

### AGENT-01: Agent executes initial message

The agent processes the task's initial message, performing code changes in the workspace.

* **Given** SANDBOX-01 has completed (the sandbox is ready)

* **When** waiting for the agent to complete its first turn

* **Then** the agent has completed its first turn
  * **And** the agent produced at least one message during execution

---

### AGENT-01.1: Agent state transitions

The agent's state reflects its execution lifecycle.

* **Given** AGENT-01 has completed

* **When** checking the sandbox status

* **Then** the agent is idle

---

### AGENT-01.2: Agent lifecycle notifications

The agent emits a complete set of lifecycle notifications during execution.

* **Given** a live event connection is established for the task

* **When** the agent completes its first turn

* **Then** a started notification is received
  * **And** one or more agent message notifications are received
  * **And** a completed notification is received
  * **And** a turn-active notification is received before the turn-completed notification
  * **And** a turn-completed notification is received

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
  * **And** a second turn-completed notification is received

---

### AGENT-02.1: Agent re-activates on follow-up

Sending a follow-up message transitions the agent from idle back to running.

* **Given** AGENT-02 message has been sent

* **When** checking the sandbox status

* **Then** the agent transitions to running and then back to idle

---

### AGENT-02.2: Second turn-completed notification

The second turn produces its own complete set of lifecycle notifications.

* **Given** a live event connection is established for the task

* **When** the agent completes its second turn

* **Then** the total count of turn-completed notifications is at least 2
  * **And** the total count of agent-completed notifications is at least 2

---

### AGENT-02.3: Files reflect second-turn changes

The workspace reflects modifications from the second agent turn.

* **Given** AGENT-02 has completed

* **When** searching workspace files for the content requested in the follow-up message

* **Then** at least one file contains the updated content

---

## Chain 2: Plan Mode

Agent execution in plan mode: the agent produces a plan for user approval before making changes.

Each scenario below depends on its parent as a precondition.

---

### TASK-01: Create task in plan mode

A task can be created with plan mode enabled, instructing the agent to plan before executing.

* **Given** a valid repository exists

* **When** the user creates a task with a repository, a message, and plan mode enabled

* **Then** the task is created in an open state

---

### SANDBOX-01: Sandbox provisions

*(Reuses SANDBOX-01 — see [sandbox-operations.md](sandbox-operations.md) for full provisioning details.)*

* **Given** the plan-mode task has been created

* **When** checking the sandbox status

* **Then** the sandbox becomes ready

---

### AGENT-03: Plan mode flow

The agent enters plan mode: it analyzes the request and produces a plan instead of immediately making changes.

* **Given** SANDBOX-01 has completed and the agent has started processing

* **When** waiting for the agent to pause for approval

* **Then** the agent produces plan content
  * **And** the agent enters a pending-response state (waiting for user input)

---

### AGENT-03.1: Agent produces plan for approval

The plan is visible in the conversation and the agent waits for user approval.

* **Given** AGENT-03 has completed (agent is waiting for approval)

* **When** retrieving the task's messages

* **Then** the messages include the agent's plan content
  * **And** a pending-response notification was received

---

### AGENT-03.2: User approves, agent executes

Approving the plan allows the agent to proceed with code changes.

* **Given** AGENT-03.1 has completed (plan is visible)

* **When** the user sends an approval message

* **Then** the agent resumes execution
  * **And** a turn-completed notification is received
  * **And** workspace files reflect the changes described in the plan

---

## Chain 3: Agent Stop, Resume & Recovery

Stopping an agent mid-execution, resuming after a sandbox restart, and recovering from agent errors.

Each scenario below depends on its parent as a precondition.

---

### TASK-01: Create task with message (agent starts)

*(Reuses TASK-01 — see [task-crud.md](task-crud.md) for full creation details.)*

* **Given** a valid repository exists

* **When** the user creates a task with a repository and a message

* **Then** the agent starts executing (wait for the agent to become active)

---

### AGENT-04: Stop agent mid-execution

A running agent can be stopped by the user before it completes.

* **Given** TASK-01 has completed and the agent is running

* **When** the user stops the task

* **Then** the agent returns to idle
  * **And** a turn-stopped notification is received

---

### AGENT-04.1: Sandbox remains ready

Stopping the agent does not affect the sandbox — it remains provisioned and healthy.

* **Given** AGENT-04 has completed

* **When** checking the sandbox status

* **Then** the sandbox is ready
  * **And** the sandbox is healthy

---

### AGENT-04.2: Agent-stopped notifications

The agent stop emits lifecycle notifications.

* **Given** a live event connection is established for the task

* **When** AGENT-04 is executed

* **Then** an agent-stopped notification is received
  * **And** a turn-stopped notification is received

---

### AGENT-06: Resume after sandbox restart

After stopping the agent, the sandbox can be stopped and restarted, and the agent resumes with full conversation continuity.

* **Given** AGENT-04 has completed (agent stopped, sandbox ready)

* **When** the sandbox is stopped, restarted, and a follow-up message is sent

* **Then** the agent completes a new turn with access to the full conversation history and prior workspace files

---

### AGENT-06.1: Sandbox stops

* **Given** AGENT-04 has completed (agent stopped, sandbox ready)

* **When** the user stops the sandbox

* **Then** the sandbox transitions to stopped

---

### AGENT-06.2: Sandbox restarts with files intact

* **Given** AGENT-06.1 has completed (sandbox stopped)

* **When** the user starts the sandbox

* **Then** the sandbox becomes ready
  * **And** files from before the stop are still present

---

### AGENT-06.3: Agent resumes with follow-up message

* **Given** AGENT-06.2 has completed (sandbox ready again)

* **When** the user sends a follow-up message requesting changes

* **Then** the agent starts a new turn and completes it

---

### AGENT-06.4: Agent retains conversation history

* **Given** AGENT-06.3 has completed (resumed turn done)

* **When** retrieving the task's messages

* **Then** the history includes the original message, agent response from before restart, follow-up message, and agent's new response
  * **And** there are at least two user messages

---

### AGENT-06.5: Files reflect resumed-session changes

* **Given** AGENT-06.3 has completed (resumed turn done)

* **When** searching workspace files

* **Then** files contain the content requested in the follow-up message

---

### AGENT-05: Agent error sets task to error (skipped)

When the agent encounters an unrecoverable error, the task reflects the error state.

* **Given** AGENT-04 has completed (agent is idle, sandbox is ready)

* **When** a message triggers an agent error (e.g., via an invalid configuration or a deliberately failing command)

* **Then** the agent returns to idle
  * **And** an agent-error notification is received
  * **And** a turn-error notification is received

---

### AGENT-05.1: Error visible in chat (skipped)

Agent errors are surfaced as messages in the conversation history.

* **Given** AGENT-05 has completed

* **When** retrieving the task's messages

* **Then** the messages include an error message from the agent

---

### AGENT-05.2: Recovery via new message (skipped)

A task that encountered an agent error can be recovered by sending a new message.

* **Given** AGENT-05 has completed (agent is idle after error)

* **When** the user sends a new message

* **Then** the agent starts a new turn
  * **And** the agent transitions to running
  * **And** a turn-active notification is received

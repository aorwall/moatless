# Agent Execution Scenarios

This spec defines all testable scenarios for agent execution: happy-path execution and multi-turn conversation, plan mode, stopping an agent mid-execution, resuming after a sandbox restart, and error recovery.

For related scenarios, see:
- [Task scenarios](../tasks/01-task-scenarios.md) — task management
- [Sandbox scenarios](../sandbox/01-sandbox-scenarios.md) — sandbox provisioning and lifecycle
- [File scenarios](../sandbox/02-file-scenarios.md) — standalone file operations

---

## Overview

### Chain 1: Agent Execution & Multi-Turn

```
├── AGENT-01: Agent executes initial message
│   ├── AGENT-01.1: Agent state transitions
│   ├── AGENT-01.2: Agent lifecycle events emitted
│   ├── AGENT-01.3: Conversation history available
│   ├── FILE-06: Files reflect agent changes
│   └── AGENT-02: Multi-turn conversation
│       ├── AGENT-02.1: Agent re-activates on follow-up
│       ├── AGENT-02.2: Second turn completion event
│       └── AGENT-02.3: Files reflect second-turn changes
```

### Chain 2: Agent Stop & Resume

```
├── AGENT-04: Stop agent mid-execution
│   ├── AGENT-04.1: Sandbox remains ready after agent stop
│   └── AGENT-04.2: Agent stopped event emitted
├── AGENT-06: Resume after sandbox restart
│   ├── AGENT-06.1: Sandbox stops
│   ├── AGENT-06.2: Sandbox restarts with files intact
│   ├── AGENT-06.3: Agent resumes with follow-up message
│   ├── AGENT-06.4: Agent retains conversation history
│   └── AGENT-06.5: Files reflect resumed-session changes
├── AGENT-05: Agent error handling (skipped)
│   ├── AGENT-05.1: Error visible in chat
│   └── AGENT-05.2: Recovery via new message
```

### Chain 3: Plan Mode

```
├── AGENT-03: Plan mode flow
│   ├── AGENT-03.1: Agent produces plan for approval
│   └── AGENT-03.2: User approves, agent executes
```

### Standalone

```
├── AGENT-07: Answering the agent's question resumes the same turn
├── AGENT-08: Subagent messages name the tool call that spawned them
└── AGENT-09: A replaced sandbox resumes the session before answering
    ├── AGENT-09.1: … under Claude
    └── AGENT-09.2: … under Codex
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

### AGENT-01.1: Agent state transitions

After completing a turn, the agent transitions to an idle state.

* **Given** AGENT-01 has completed

* **When** checking the agent's state

* **Then** the agent state has settled to idle

---

### AGENT-01.2: Agent lifecycle events emitted

Real-time events are emitted during agent execution.

* **Given** AGENT-01 has completed

* **When** reviewing the real-time events for the task

* **Then** agent started, message, and completed events were emitted
  * **And** turn active and turn completed events were emitted

---

### AGENT-01.3: Conversation history available

The conversation history reflects the messages after agent execution.

* **Given** AGENT-01 has completed

* **When** retrieving the task's messages

* **Then** the response contains at least the initial user message
  * **And** the response contains at least one assistant message

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

### AGENT-02.1: Agent re-activates on follow-up

After the second turn, the agent returns to an idle state.

* **Given** AGENT-02 has completed

* **When** checking the agent's state

* **Then** the agent state has settled to idle

---

### AGENT-02.2: Second turn completion event

A second set of completion events is emitted for the follow-up turn.

* **Given** AGENT-02 has completed

* **When** reviewing the real-time events for the task

* **Then** at least two agent completed events have been emitted
  * **And** at least two turn completed events have been emitted

---

### AGENT-02.3: Files reflect second-turn changes

The workspace reflects modifications from the second agent turn.

* **Given** AGENT-02 has completed

* **When** searching workspace files for the content requested in the follow-up message

* **Then** at least one file contains the updated content

---

## Chain 2: Agent Stop & Resume

Stopping an agent mid-execution, resuming after a sandbox restart, and recovering from errors.

Each scenario below depends on its parent as a precondition. A task with an initial message is created first, and the agent starts executing.

---

### AGENT-04: Stop agent mid-execution

A running agent can be stopped by the user before it completes.

* **Given** a task has been created and the agent is running

* **When** the user stops the task

* **Then** the agent stops executing
  * **And** the agent becomes idle

---

### AGENT-04.1: Sandbox remains ready after agent stop

The sandbox environment persists after the agent is stopped.

* **Given** AGENT-04 has completed

* **When** checking the sandbox status

* **Then** the sandbox is still in a ready or stopped state

---

### AGENT-04.2: Agent stopped event emitted

A real-time event is emitted when the agent is stopped.

* **Given** AGENT-04 has completed

* **When** reviewing the real-time events for the task

* **Then** an agent stopped or turn stopped event has been emitted

---

### AGENT-06: Resume after sandbox restart

After stopping, the sandbox can be restarted and the agent resumes with full conversation continuity.

---

### AGENT-06.1: Sandbox stops

The sandbox is stopped as part of the restart cycle.

* **Given** AGENT-04 has completed (agent stopped)

* **When** the user stops the sandbox

* **Then** the sandbox transitions to stopped

---

### AGENT-06.2: Sandbox restarts with files intact

The sandbox restarts with workspace files preserved.

* **Given** AGENT-06.1 has completed

* **When** the user starts the sandbox

* **Then** the sandbox becomes ready
  * **And** workspace files from before the restart are still present

---

### AGENT-06.3: Agent resumes with follow-up message

A follow-up message triggers a new agent turn after restart.

* **Given** AGENT-06.2 has completed (sandbox is ready)

* **When** the user sends a follow-up message

* **Then** the agent completes a new turn

---

### AGENT-06.4: Agent retains conversation history

The conversation history spans the entire session, including messages from before and after the restart.

* **Given** AGENT-06.3 has completed

* **When** retrieving the task's messages

* **Then** the conversation history includes messages from before and after the restart
  * **And** there are at least two user messages in the history

---

### AGENT-06.5: Files reflect resumed-session changes

The workspace files show changes made during the resumed session.

* **Given** AGENT-06.3 has completed

* **When** searching workspace files for the content requested in the follow-up message

* **Then** at least one file contains the expected content

---

### AGENT-05: Agent error handling (skipped)

When the agent encounters an error, the task reflects the error state.

> **Note:** This scenario is currently skipped — it requires the ability to inject an error-causing message.

* **Given** the agent is running

* **When** the agent encounters an error during execution

* **Then** the agent reports the error and returns to idle

---

### AGENT-05.1: Error visible in chat

The error from agent execution is visible in the conversation history.

* **Given** AGENT-05 has completed

* **When** retrieving the task's messages

* **Then** the conversation includes an error message

---

### AGENT-05.2: Recovery via new message

After an error, the agent can recover by processing a new message.

* **Given** AGENT-05 has completed

* **When** the user sends a new message

* **Then** the agent activates for a new turn
  * **And** the agent can successfully execute

---

## Chain 3: Plan Mode

Agent execution in plan mode: the agent produces a plan for user approval before making changes.

---

### AGENT-03: Plan mode flow

The agent pauses to present a plan before making changes.

* **Given** a task has been created with a message requesting a plan and the sandbox is ready

* **When** waiting for the agent to process the message

* **Then** the agent pauses for approval or completes if plan mode is not enforced

---

### AGENT-03.1: Agent produces plan for approval

The agent produces plan content for the user to review.

* **Given** AGENT-03 has completed

* **When** retrieving the task's messages

* **Then** the conversation includes assistant content
  * **And** if plan mode is active, a pending response event has been emitted

---

### AGENT-03.2: User approves, agent executes

After approval, the agent proceeds to implement the plan.

* **Given** AGENT-03.1 has completed

* **When** the user approves the plan (or the agent has already completed)

* **Then** at least one turn completion event has been emitted

---

## Standalone

### AGENT-07: Answering the agent's question resumes the same turn

An agent that needs a decision pauses and asks. The answer has to reach the paused conversation, so that the agent carries on from where it stopped instead of starting over or waiting forever.

* **Given** a task whose message asks the agent to put a question to the user
  * **And** the sandbox is ready

* **When** the agent pauses to ask its question
  * **And** the user answers it

* **Then** the agent resumes and finishes the turn
  * **And** the answer is part of the conversation

If the agent finishes without asking — which the model is free to do — the scenario asserts only that the turn completed.

---

### AGENT-08: Subagent messages name the tool call that spawned them

Work an agent delegates to a subagent has to be attributable, so the conversation can show it under the step that started it rather than mixed into the main thread.

* **Given** a task whose message asks the agent to delegate work to a subagent
  * **And** the sandbox is ready

* **When** the agent's turn completes

* **Then** the delegating step has a transcript of the subagent's work
  * **And** none of that work appears in the main conversation, which shows the delegating step alone

If the agent does not delegate — which the model is free to do — the scenario is skipped rather than passing without having checked anything.

---

### AGENT-09: A replaced sandbox resumes the session before answering

A message sent while the sandbox is down is kept, and a replacement sandbox is started to answer it. The replacement must restore the conversation before it accepts that message — otherwise the agent answers with no memory of what came before.

Run once per harness (AGENT-09.1 Claude, AGENT-09.2 Codex): each restores its session its own way, and only output the harness actually produced proves it ran.

* **Given** a task whose first turn has completed
  * **And** the user has stopped the sandbox

* **When** the user sends a message while nothing is running to receive it
  * **And** a replacement sandbox is provisioned to answer it

* **Then** the agent continues the same conversation it had before
  * **And** the waiting message is answered exactly once, against that conversation
  * **And** the reply comes from the harness the task was created with

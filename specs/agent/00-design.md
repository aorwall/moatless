# Agent Execution, Messages & Commands

The agent is the AI assistant that processes user messages, makes code changes, and converses within a task's sandbox. Commands provide reusable prompts that can be attached to tasks.

## What Users Can Do

### Agent Execution
- **Start the agent** — happens automatically when a message is sent to a task with a ready sandbox.
- **Watch execution** — see the agent's progress in real-time as it works through a turn.
- **Stop the agent** — interrupt execution mid-turn; the agent halts and can be resumed later.
- **Resume after restart** — restart the sandbox and send a new message; the agent picks up with full conversation history.
- **Multi-turn conversation** — send follow-up messages after the agent completes a turn; each message triggers a new turn.

### Messages
- **View conversation history** — see all user and assistant messages in chronological order.
- **Inspect content blocks** — messages can contain text, code, tool calls, and other structured content.
- **Track cost** — each turn's token usage is recorded; the task's total cost accumulates over turns.

### Commands
- **Create reusable prompts** — define commands with a name, prompt template, and optional conditions.
- **Attach commands to tasks** — commands can be triggered by name during a conversation.

## How It Works

1. When a user sends a message, the agent activates for a **turn** — one round of work that may involve multiple tool calls and file changes.
2. The agent runs inside the task's sandbox and has access to the workspace files.
3. Stopping the agent mid-turn is graceful: the conversation history is preserved up to the interruption point.
4. After a sandbox restart, the agent resumes with the full message history — no context is lost.
5. Commands are stored per-repository and expand into the conversation when invoked.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Turn | One round of agent work in response to a user message |
| Idle / Running | The two agent states — idle waits for input, running processes a turn |
| Content block | A unit within a message: text, code, tool use, or result |
| Command | A reusable prompt template that can be triggered by name |
| Plan mode | A special mode where the agent proposes changes before making them |

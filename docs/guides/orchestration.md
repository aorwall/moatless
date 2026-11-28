# Orchestration

This guide shows how one task hands work to others, waits for them without polling, and keeps its place across retries and restarts. It is for people writing skills or Loops that split work across tasks. For the Loop file format, see [loops](loops.md).

## 1. When to create a new task

A new Moatless task has its own sandbox, its own agent session and its own conversation. Create one when the work needs one of those:

- **A different environment.** The change touches another repository, or its check needs Docker, a preview, a browser or more memory than your sandbox has. Pick the workspace that can run it.
- **Someone to talk to.** You or a person want to follow the work, ask it questions or steer it while it runs. A task is a thread anyone with access can open and chat with, and it stays readable after it finishes.
- **Work that outlives your turn.** A fix that waits on review, CI or a merge can sit in its own task and report back when something happens.
- **Parallel work in separate checkouts**, so two agents never edit the same files.

Use your harness's own subagents instead when you need none of that. Claude Code and Codex can start subagents inside the same session to search code, read many files or review a diff in parallel. They share your sandbox and checkout, cost no new pod, and hand back a summary within your turn. Nobody can chat with them and they vanish with the turn, so keep them for research and checks you could run yourself.

Decide before you install dependencies or start the build, and say in your reply why you delegated.

## 2. Parent and child

Create a child with `moat tasks create`. Inside a sandbox your own task is always the parent. Outside one, pass `--parent <task-id>`.

```bash
moat tasks create --workspace <workspace> --agent-profile claude-code \
  --name "<what it does>" --context-file /tmp/brief.md \
  --message "Read the attached brief and do the task."
```

The backend wakes the parent each time the child's turn completes or errors. The notice carries no result, so the child sends its result with `moat send --task <parent>`: what it did, links, how it checked, and any question. A skill can ask for a fixed result block so the parent can parse it.

Pick the workspace that can run the child's checks. `--workspace` is required unless a parent task or a joined sandbox settles one.

### Forks, subtasks and same-sandbox children

| Relation | Created by | Close cascade | Context |
|---|---|---|---|
| Subtask (`created_by`) | `moat tasks create` from a task | Closing the parent can close it | Only what the brief says |
| Subtask in the parent's sandbox | `moat tasks create --same-sandbox` | Same as a subtask | Its own session in the parent's checkout. Under Claude Code it is seeded with the parent's context; otherwise it starts with none |
| Fork | Branching a task at a completed turn | The parent's close cascade and error notifications skip it | Continues the parent's trajectory from the cut |

The checkout is `shared` by default, and `--checkout-mode isolated` gives a task its own checkout in the same pod. Two agents in one shared checkout collide.

Closing a task that has children needs an explicit choice:

```bash
moat tasks close <task-id> --close-children true
```

A closed task's bindings stop. Close a child you are replacing before its successor starts answering the same thread, or both answer the same comment.

## 3. Waiting without polling

After you start work, end your turn. Do not sleep or loop on status. Something wakes you:

- a child's turn ends or errors;
- a `moat cmd run` command exits or hits its deadline (a `moat cmd kill` of your own wakes nothing);
- a message arrives, such as a person's reply or a CI failure.

A task that is bound to its pull request, including one that opened it with `moat gh pr create`, gets failed checks as messages. It needs no watcher. Start long work with `moat cmd run`, because only a registered command keeps the sandbox awake; the `moat-cli` skill has the details.

A task that is not your child sends you nothing. This covers a predecessor's children, a PR owner you asked for help, and tasks another Loop started. Read them when you need them:

```bash
moat tasks status <task-id> --result
```

`--result` shows whether the task is working, waiting for input or for its own children, or done, and its latest result.

## 4. The ledger pattern

A long-running orchestrator outlives its context and sometimes its own task, so its conversation is not a safe record. Keep a ledger in `moat memory`, in the Loop's scope, with one entry per unit of work such as a ticket or a branch. It holds what the outside world does not: which child plays which role, which comments you handled, what you created.

Each turn:

1. **Load the ledger.** Read content and version from one response.
2. **Re-read live state.** Read the ticket, PR or alert again. Act on what is true now, not on what the waking message says, because messages queue and a child may have moved things since.
3. **Act.** Make each step safe to repeat: doing the same state twice must change nothing.
4. **Write the ledger** before the turn ends.

```bash
moat memory get <key> --json
moat memory put <key> --if-version <n> --no-expiry --summary "<one line>" --data '{...}'
```

- **Write the child id before briefing the child.** Record it as soon as `moat tasks create` returns, in the same turn. A ledger without it starts a second child on the next message. One way: create the child without `--message`, write the id with `"briefed_at": null`, then send the brief. A later turn that finds an open child with no brief time sends it, so a turn that died halfway recovers.
- **Pass `--if-version`.** `0` means the entry must be new. A 409 means another turn wrote first: re-read and reapply your change to the latest content.
- **Never fall back to an empty ledger.** When a read or parse of an existing entry fails, stop. Writing `{}` would erase the children it names. Rebuild only when the entry is truly missing.
- **Write the whole object.** A put replaces the data and does not merge fields.

An entry expires 28 days after its last put unless you pass `--no-expiry`. Rebuild a missing ledger from `moat tasks children <id>` and each child's `--result`. Name children by a rule, such as `<ticket> - <role> - <title>`, so this works.

### A successor adopts its predecessor's children

A Loop closes an idle task and starts a new one on the next event. The new task has no children, so an empty `moat tasks children` means "you cannot see them", not "none were started". Find the earlier task through the ledger, or list tasks by name as the alert monitor does. Record your own id, read each open child's result, and tell the child who its parent is now, because its reports to the old task went nowhere.

## 5. Coordinating independent tasks

Tasks without a shared parent still meet over the same ticket, branch or alert.

**Memory claims work as locks.** `moat memory claim <key> --ttl 2h` fails while another task holds the key, and `moat memory release <key>` drops it. A claim is advisory: it stops another claim, not a plain write.

**Fingerprints deduplicate.** Give each problem a stable identity, such as an alert fingerprint or a repo and branch pair. Use it in task names, ledger keys and replies. Before you start a fix, look for an open task with that name across all tasks, not only your children. A match gets the new evidence with `moat send --task <id> --queue`, and no new task. A related cause counts too: two alerts on one pod share a fix.

**The task that wrote a change owns its breakage.** Moatless appends a `Moatless task: <link>` footer to the pull requests and ticket comments a task posts. A dispatcher that sees a build fail on `main` reads the footer on the commit's PR. If that task is open, the dispatcher sends it the failure and asks for a result block, and starts no new fix task, because the owner has the context. If the owner reports the failure is not its change, the dispatcher starts a fresh fix task.

## 6. Handoffs

A handoff moves work from your session to a new task. The next agent starts with only the message and attached files, so put the context in them:

- the objective and what the next task does;
- decisions made and the constraints behind them;
- repo state, with branch, PR and issue links;
- files to read first, verification done and still owed, and open questions;
- the skills it should load, since `moat tasks create` has no flag for that.

Attach long material with `--context-file` and redact secrets.

```bash
# inside a sandbox: inherit this checkout, including uncommitted work
moat tasks create --same-sandbox --name "Handoff: <focus>" \
  --agent-profile claude-code --headless false \
  --message "$HANDOFF" --context-file path/to/file

# outside a sandbox: choose a workspace
moat tasks create --workspace <workspace> --name "Handoff: <focus>" \
  --agent-profile claude-code --message "$HANDOFF"
```

Stop working once the new task exists. `--headless false` keeps the question and plan tools so a person can answer; drop it when nobody will watch.

## 7. Agents messaging each other

Any task can message any other task it can access, not only its parent or children:

```bash
moat send --task <task-id> --text "<outcome and what to act on>"
moat send --task <task-id> --queue --text "<new evidence>"
```

The message starts or steers a turn in the recipient's agent. Without `--queue` it steers into the turn already running. With `--queue` it waits for that turn to end and opens the next one. `--effort` sets the reasoning effort for the turn it starts.

The recipient is an agent, so send one message per finished piece of work: the outcome and what to act on, with no acknowledgements or progress updates. Name your own task id when you want a reply, since the recipient answers with its own `moat send --task`.

Typical uses:

- a child reports its result or asks its parent a question;
- a parent sends a child a correction or a person's reply;
- a monitor passes new evidence to the task already fixing that problem, instead of starting a second one;
- a successor tells an adopted child who its parent is now.

To read another task without waking it, use `moat tasks status <task-id> --result`.

## 8. Patterns

| Pattern | Shape | In the moatless repo |
|---|---|---|
| Orchestrator | One long-lived task per ticket or branch. It reads the subject's state and starts the child for the next step. The ledger records which child plays which role. | |
| Capped fan-out fixer | It investigates read-only, then explains the problem or starts a fix child that opens a PR. Only a few fix tasks work at once, and a task whose PR waits for review does not count. | The `alerts-moatless-monitor` Loop with the `alert-fix` skill |
| Approval-gated fan-out | A scheduled run groups the work and publishes it for approval. Only approved groups become tasks, and each closes its items when its PR merges. | The `feedback-triage` Loop |

See [Moatless fixing itself](../use-cases/moatless-fixing-itself.md) and [self-improvement](self-improvement.md) for the second and third. One detail is worth copying. The alert monitor starts every brief with the Slack message id and the fingerprint, and the child repeats both in each report, so the monitor can reply in the right thread after its own context is summarised.

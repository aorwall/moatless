---
name: moat-cli
description: "Use the moat CLI for authentication, tasks, repositories, workspaces, loops, messages, and sandbox status. Use when creating or operating tasks or checking current command syntax."
---

# Moat CLI

Run `moat --help` and the relevant subcommand's `--help` before composing an
unfamiliar command. The current parser is in [main.rs](../../../crates/cli/src/main.rs)
and [commands](../../../crates/cli/src/commands). If the installed binary differs,
build `cargo build -p moatless-cli --bin moat` and use `target/debug/moat`.

Authenticate with `moat login`; use `moat status` to verify the active host and
identity. `MOATLESS_API_URL` selects the API host, and `MOATLESS_API_KEY` supplies
an API key. Do not print credentials when inspecting configuration.

```bash
moat repos list --json
moat workspaces list --json
moat tasks create --workspace <workspace-id> --agent-type claude-code \
  --headless false --name "Sandbox smoke"
moat tasks get <task-id> --json
moat tasks status <task-id> --json
moat tasks logs <task-id> --no-follow --tail 500
moat messages list <task-id> --full
```

Task creation provisions the sandbox. Supplying `--message` also delivers the
initial message; without it there is no initial agent turn. Task `status` is
`open`, `closed`, or `error`; agent completion does not close the task. Use
agent state and messages to determine whether a turn completed.

Tasks may share a sandbox. Read `sandboxId` and `workspaceId` from task details
before operating infrastructure or changing configuration. Select `--workspace`
when a task needs a configured workspace; `--repo` may select its bare workspace.
`--agent-type` is required, and `--headless` defaults to true.

## Running a command that outlives one tool call

`moat cmd run` hands a long command to the sandbox, which owns the process and
stays awake until it finishes. `moat cmd list|status|logs|kill` operate it.
`moat cmd run --help` documents the flags, the working directory default, and
the 4h deadline. What follows is the part `--help` does not state.

**You are woken when the command reaches any terminal state**: exited, timed
out, or killed. The wake is unconditional, and it also covers a command that
died before the sandbox armed its watch. Do not poll for the outcome, and do not
schedule a fallback wakeup.

**Read the outcome back** with `moat cmd status` or `moat cmd logs`. Until you
do, the sandbox still counts the command as owed to you. Killing it counts as
reading it.

**Only a registered command holds the sandbox open.** A process you fork from a
Bash tool call with `&` is invisible to the idle reaper, which will stop the pod
underneath it. Start anything that must outlive the turn through `moat cmd run`.

**Detach what you do background**, for the rest of a turn:

```bash
setsid nohup CMD >log 2>&1 </dev/null & disown
```

`&` alone leaves the child in this tool call's session and process group, where
anything aimed at the group reaches both. `setsid` gives it its own, the
redirects release the call's stdio, and `disown` drops it from the job table.

**`pkill -f <pattern>` kills the tool call that runs it** whenever the pattern
also matches the call's own command line, which holds your whole command as
text: `pkill -f "sleep 24"` returns exit 144 and the output is lost. Match on
something the wrapper cannot contain, or kill by pid from `pgrep -f <pattern>`,
dropping this shell's own `$$` from the list — `pgrep` excludes itself and not
its caller.

**A pod replacement destroys every running command and its logs.** The sandbox
announces the loss once, on the next turn, and names the commands that died.
Treat that message as the outcome you were waiting for, and start the work
again if it still matters.

## Restarting your own sandbox

`moat tasks restart $TASK_ID --yes` and `moat tasks redeploy $TASK_ID --yes`
replace the pod you are running in. Without `--yes` both refuse and print what
it costs. Use a restart for a wedged pod, and a redeploy to pick up a new image
or a changed repository configuration.

**The command never returns.** The pod dies while the request is in flight, so
this `moat` process and your current turn die with it. Say what you need to say
before you run it.

**You are woken once the pod is back.** The new pod resumes your session and
sends you a `<sandbox-restart-note>` naming the operation you ran. Treat that as
the outcome, and continue from there.

**Gitignored build artifacts do not survive.** The workspace is restored from a
snapshot, so re-run install and build steps before you rely on compiled output.

**A restart that cannot leave that note is refused.** The wake is recorded
inside the pod before the request goes out, so a pod too wedged to answer its
own gRPC server stops the restart rather than replacing the pod silently. The
command prints the failure. Ask a person to restart the task from outside when
you see it.

Read [command workflows](references/command-reference.md) for response shapes,
workspace ownership, task reuse, and bounded diagnostics. Read
[Loops](references/loops.md) when installing or operating automation.

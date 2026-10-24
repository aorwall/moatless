# Which repository owns the fix

`reportedFrom` is the repository the reporting agent was checked out in. Name
the owner from what the comment says has to change, and use `reportedFrom` only
when nothing else in the comment points anywhere.

| The comment is about | Owner |
|---|---|
| The sandbox itself: restarts, evictions, lost `target/` or working tree, the clone, missing daemons, toolchains or services in the image, CPU or memory limits | `moatless` |
| The `moat` CLI or a backend endpoint: a flag, an error, an output shape, `moat gh`, `moat cmd`, `moat feedback`, a LOOP.md field that fails to parse | `moatless` |
| A skill in the `moatless-skills` or `moatless-admin` plugin, such as `artifacts` or `daily-health-report` | `moatless` (`plugins/`) |
| A skill in the `soaplabs` marketplace, named `<plugin>:<skill>` with a plugin like `general`, `agentops`, `backend`, `frontend`, `operations`, `infra` or `product` | `soap-knowledge` (`plugins/<plugin>/skills/`) |
| A skill or loop that lives in one repository, under `.claude/skills/`, `.agents/skills/` or `.moatless/loops/` | That repository |
| A repository's `CLAUDE.md` or `AGENTS.md`, its build, lint, tests or CI | That repository |
| The T3 Code UI, or the `fork-upstream-merge` skill | `t3code` |

Find where a named skill lives when the comment does not say:

```bash
moat gh api 'search/code?q=<skill-name>+in:path+org:soaplabs' --jq '.items[] | "\(.repository.name) \(.path)"'
```

One comment can name two fixes, such as a skill snippet that is wrong and a CLI
flag that misbehaves. Put the item in the cluster of its main complaint and
mention the second fix in that cluster's brief.

Write the reason beside the owner in `ownerReason` whenever the owner differs
from `reportedFrom`, such as *the clone depth is set by the sandbox, not the
repo*. A person correcting a wrong owner reads that reason first.

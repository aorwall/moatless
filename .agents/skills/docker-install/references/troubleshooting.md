# Docker installation failures

| Symptom | Check and repair |
|---|---|
| Port occupied | Read `moat docker preflight --json`; configure the appropriate port with `moat docker env set`, then rerun install. |
| Image cannot execute | Inspect the configured image architecture. Build the sandbox locally on Apple Silicon using [build-sandbox.sh](../../../../docker/build-sandbox.sh) and its supported `--only` options. |
| Pull denied or manifest missing | Check registry access, `APP_VERSION`, and `SANDBOX_IMAGE`; use matching published application tags. The published sandbox is `aorwall/moatless-sandbox`, with a `moatless-sandbox-js` alias. |
| Backend rejects encryption key | Recover the original valid key for an existing database. A new key cannot decrypt stored secrets. Let init generate a key only for a fresh installation. |
| Agent reports not logged in | Check the selected agent's credential source and expiry. Refresh its credential and recreate affected containers; inspect sandbox logs. This message does not diagnose a system encryption-key failure. |
| Changed environment is absent | Run `moat docker install` to recreate containers. Restarting an existing container preserves its original environment. |
| Postgres or migration failure | Read backend and Postgres logs and the status endpoint details. Check connectivity and migration error before changing data. |
| Preview unavailable | Expected under Docker: `DockerSandboxAdapter` does not publish a Script's `port`, so a Script with a `port` has no preview URL. |

Use `moat docker env list --json` for masked inspection. Preserve system encryption
keys and data volumes during repairs. Purging an installation is a data-removal
operation, not a generic health fix.

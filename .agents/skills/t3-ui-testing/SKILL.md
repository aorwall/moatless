---
name: t3-ui-testing
description: Test the T3 Code UI against a deployed Moatless backend in a real browser with Playwright. Covers signing a headless browser in with an API key, starting a thread, running a project Script and checking its preview, archiving, and running or extending the tests/ui suite. Use when asked to click through, screenshot, smoke-test or verify T3 Code or Moatless "in the browser", "with playwright" or "end to end in the UI", on any deployment. Not for the CLI-driven tests/api chains (e2e-testing) or the local two-repository dev setup (t3-development).
---

# Test T3 Code against Moatless in a browser

This skill drives the deployed T3 client and its Moatless backend from Playwright. The client and the API share one origin, written `$MOATLESS_URL` below (for example `https://moatless.example.com`), and each task's previews live on `https://<task-id>--<port>.preview.<domain>`.

## Sign the browser in

Deployed login pages only offer OAuth, which a headless browser cannot complete. Trade the sandbox's `MOATLESS_API_KEY` (an `mvk_` key) for a session cookie instead:

```bash
node .agents/skills/t3-ui-testing/scripts/mint-state.mjs --url "$MOATLESS_URL" --out /tmp/moatless-auth.json
playwright-cli open && playwright-cli state-load /tmp/moatless-auth.json
playwright-cli goto "$MOATLESS_URL/"
```

The script posts to `/api/v1/auth/api-key/session` and writes `moatless_session` and `moatless_csrf` as `.<parent-domain>` cookies, so preview hosts are signed in too. Landing on `/login` means the cookie did not apply.

## Run the suite

`tests/ui` holds the browser suite. It runs against a deployment, not the `scripts/local-e2e` harness, and every run spends two real agent turns on the project:

```bash
moat cmd run --cwd <moatless checkout> "E2E_UI_URL=$MOATLESS_URL bun run test:e2e:ui"
```

| Variable | Effect |
|---|---|
| `E2E_UI_URL` | Deployment origin. Falls back to `FRONTEND_URL`. |
| `MOATLESS_API_KEY` | Signs the browser in and authorises the REST reads in `tests/ui/helpers/backend.ts`. |
| `E2E_UI_PROJECT` | Project to start the thread on. Defaults to `Moatable`. |

`t3-thread-build-flow.spec.ts` is the template for a new chain. The browser does every action, and the REST helper only answers what the UI does not state reliably: whether a turn finished, and which Scripts a task holds. A failed chain closes its task in `afterAll`. Failures leave a screenshot and a trace under `test-results/ui/artifacts/`.

## Where things are in the UI

| Action | Locator |
|---|---|
| New thread | `button "New thread"` with `exact: true`, because a sidebar row can also be titled "New thread, <project>" |
| Pick the project | `dialog "Command palette"`, then `option` whose name starts with `<Project> ` |
| Send | `textbox "Message"`, then press Enter |
| Run a Script | `button "Run <label>"` in the thread panel's `Project scripts` group |
| Preview | `iframe[title="Preview"]` |
| Archive | `button "Thread actions for <title>"` → `Archive thread` → the `alertdialog`'s `Confirm` button, which only appears when archive confirmation is on in settings |

A thread page lives at `/<environment-id>/<task-id>`. Opening one from the sidebar always uses the task id.

## Gotchas

- **Read the task id only after the URL leaves `/draft/`.** A draft lives at `/draft/<draft-id>`, and sending it moves the page to `/<environment-id>/<thread-id>`. The backend creates the task under that thread id, so the id in the thread URL is the task id.
- **Only a Script with a port is viewable.** A server the agent starts with `moat cmd run` is reachable inside its own pod only, because traffic between pods is blocked. Start servers as Scripts.
- **"Add project script" fails with "server.updateSettings is not available on this server" on a deployment without the `projects.mutate` fix** in both the T3 client and the backend. There, declare the Script in the repository's `.moatless/workspaces.json`, run `moat repos sync-config <repo>`, then run it from the UI. A workspace Script stays for every later task on that project.
- **A Script runs in the repository checkout,** so `bun run dev` needs no `cd`. Bind Vite to `0.0.0.0` with `--strictPort`, or the preview URL points at a port nothing serves.
- **A preview pane that reads "Sandbox required — This thread is not available" on a thread you just started needs a page reload** on a T3 client without the sandbox status retry. A draft subscribes to its sandbox status before the task exists, and that client keeps the refusal. The pane then parks the preview iframe hidden at 0,0 behind the chat: the frame's content still reads, but clicks land on the chat.
- **The `Run <label>` buttons disappear while a preview is open,** because the preview covers the panel's script list. Check a Script through `/api/orchestration/scripts/<task-id>`, not through its button.
- **The first preview load can arrive before Vite is listening,** and the error page never retries by itself. Reload the iframe by setting `src` to its current value until the expected content shows.
- **Archive is the close action, and Settle is not.** Archive sends `thread.archive`, which closes the task. Settle stops the sandbox and leaves the task open. Archive is disabled while the agent is running.
- **`agentStatus: idle` does not mean the turn has finished,** because a task reports idle before its first turn starts. A turn is finished once `/api/v1/tasks/<id>/messages?summary=true` holds one `result` message per turn and `live-status` reports `idle`.
- **`playwright-cli run-code` prints the code it runs.** Never put the API key in it. Load the state file from `mint-state.mjs` instead.
- **A page from `browser.newPage()` inside `beforeAll` ignores the config's `use` block.** It starts signed out and without a base URL. Create a context with `storageState` and `baseURL` explicitly, as the template spec does.
- **`moat tasks scripts` takes the task id before the subcommand:** `moat tasks scripts <task-id> list`, not `list <task-id>`.
- **The UI warns "Client and server versions differ"** when the deployed client is ahead of the backend. Note it in your report before you blame the client for a missing feature.

## Done when

- The report names the deployment, the project and the task id each run created.
- Each check names the visible evidence, such as the heading text or the counter value, not only that a page loaded.
- Every task the run created is archived or closed, and the report lists any project Script the run added.

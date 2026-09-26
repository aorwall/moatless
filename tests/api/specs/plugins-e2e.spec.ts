/**
 * Chain: Plugins E2E
 *
 * Verifies the full plugin flow: register a plugin, sync its skills, turn it on
 * for everyone, escape it with a personal record, put it back, narrow it in one
 * workspace and put that back too, then provision a sandbox and verify the
 * agent has access to the plugin's skill — and only to the skills that were
 * picked.
 *
 * Uses: https://github.com/amekala/adspirer-mcp-plugin.git
 * Which contains skill: ad-campaign-best-practices
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   PLUGIN-01 -> PLUGIN-01.1 -> PLUGIN-02 -> PLUGIN-03 -> PLUGIN-04 -> PLUGIN-04.1 ->
 *   PLUGIN-05 -> PLUGIN-05.1 -> PLUGIN-05.2 ->
 *   TASK-01 -> SANDBOX-01 -> SANDBOX-01.1 -> AGENT-01 -> AGENT-01.1 -> CLEANUP
 */

import { budget, missingCapability } from '../../shared/profile'
import { scopedName } from '../../shared/run-scope'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { createTaskWithMessage, waitForAgentTurnComplete, waitForSandboxReady } from '../steps'

const agentReason = missingCapability('claudeCredentials')

const PLUGIN_GIT_URL = 'https://github.com/amekala/adspirer-mcp-plugin.git'
const PLUGIN_NAME = 'adspirer-mcp-plugin'
const PLUGIN_SKILL = 'ad-campaign-best-practices'

test.describe('Chain: Plugins E2E', () => {
  test.describe.configure({ mode: 'serial' })

  test.skip(agentReason !== null, agentReason ?? '')

  const ctx = new ChainContext()
  const events = chainEvents()
  let pluginId: string
  let workspaceId: string

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
    ctx.repository = repo
  })

  test.afterAll(async ({ api }) => {
    // Deleting the plugin cascades its skills and activation records away.
    if (pluginId) {
      await api.deletePlugin(pluginId).catch(() => {})
    }
    if (workspaceId) {
      await api.deleteWorkspace(workspaceId).catch(() => {})
    }
    // Stop + delete task
    await discardTask(api, ctx.taskId)
  })

  // --- PLUGIN-01: Register plugin -------------------------------------------

  test('PLUGIN-01: Register plugin', async ({ api }) => {
    const plugin = await api.createPlugin({
      name: PLUGIN_NAME,
      gitUrl: PLUGIN_GIT_URL,
    })

    expect(plugin.id).toBeTruthy()
    expect(plugin.name).toBe(scopedName(PLUGIN_NAME))
    expect(plugin.gitUrl).toBe(PLUGIN_GIT_URL)

    pluginId = plugin.id
    progress(`Registered plugin: ${plugin.id}`)
  })

  // --- PLUGIN-01.1: List shows new plugin -----------------------------------

  test('PLUGIN-01.1: List plugins includes new plugin', async ({ api }) => {
    const plugins = await api.listPlugins()
    const found = plugins.find((p) => p.id === pluginId)

    expect(found).toBeDefined()
    expect(found!.name).toBe(scopedName(PLUGIN_NAME))
  })

  // --- PLUGIN-02: Reading skills syncs the plugin ---------------------------

  test('PLUGIN-02: Reading skills syncs the plugin from its source', async ({ api }) => {
    const skills = await api.listPluginSkills(pluginId)
    expect(skills.map((s) => s.name)).toContain(PLUGIN_SKILL)
    ctx.set('skillNames', skills.map((s) => s.name).sort())

    // The sync stored a version, so the next read costs no fetch.
    const plugins = await api.listPlugins()
    const found = plugins.find((p) => p.id === pluginId)
    expect(found!.syncedVersion).toBeTruthy()

    progress(`Synced ${skills.length} skill(s)`)
  })

  // --- PLUGIN-02.1: Nothing is delivered until somebody turns it on ---------

  test('PLUGIN-02.1: A plugin nobody turned on reaches nobody', async ({ api }) => {
    const effective = await api.listEffectivePlugins()
    expect(effective.find((p) => p.pluginId === pluginId)).toBeUndefined()
  })

  // --- PLUGIN-03: Turn it on for everyone -----------------------------------

  test('PLUGIN-03: Turning it on for everyone delivers it', async ({ api }) => {
    await api.setPluginActivation(pluginId, {
      reach: 'everyone',
      enabled: true,
    })

    const records = await api.listPluginActivations(pluginId)
    expect(records).toHaveLength(1)
    expect(records[0]!.reach).toBe('everyone')
    expect(records[0]!.enabled).toBe(true)

    const effective = await api.listEffectivePlugins()
    const found = effective.find((p) => p.pluginId === pluginId)
    expect(found).toBeDefined()

    // Turning the whole plugin on delivers every skill it synced, not just the
    // one this chain later asks the agent about. Checking for that one skill
    // alone would pass on a plugin that silently delivered nothing else.
    expect(
      [...found!.skillNames].sort(),
      "the effective set is not the plugin's full skill list",
    ).toEqual(ctx.get<string[]>('skillNames'))
    expect(found!.skillNames).toContain(PLUGIN_SKILL)
  })

  // --- PLUGIN-04: A personal record overrides the global default -----------

  test('PLUGIN-04: A personal record switches the global default off', async ({ api }) => {
    await api.setPluginActivation(pluginId, {
      reach: 'personal',
      enabled: false,
    })

    const effective = await api.listEffectivePlugins()
    expect(effective.find((p) => p.pluginId === pluginId)).toBeUndefined()
  })

  // --- PLUGIN-04.1: Clearing it hands you back to the default ---------------

  test('PLUGIN-04.1: Clearing a personal record restores the default', async ({ api }) => {
    await api.clearPluginActivation(pluginId, 'personal')

    const records = await api.listPluginActivations(pluginId)
    expect(records.every((r) => r.reach === 'everyone')).toBe(true)

    const effective = await api.listEffectivePlugins()
    expect(effective.find((p) => p.pluginId === pluginId)).toBeDefined()
  })

  // --- PLUGIN-05: A workspace narrows what loads in it ----------------------

  test('PLUGIN-05: A workspace record keeps a globally-on plugin out of it', async ({ api }) => {
    const ws = await api.createWorkspace({
      name: 'plugin-scoped-workspace',
      repos: [{ repositoryId: ctx.repositoryId!, isPrimary: true }],
    })
    workspaceId = ws.id

    await api.setPluginActivation(pluginId, {
      reach: 'workspace',
      workspaceId,
      enabled: false,
    })

    const here = await api.listEffectivePlugins(workspaceId)
    expect(
      here.find((p) => p.pluginId === pluginId),
      'the workspace record must outrank the global default',
    ).toBeUndefined()

    const elsewhere = await api.listEffectivePlugins()
    expect(
      elsewhere.find((p) => p.pluginId === pluginId),
      'narrowing one workspace must not follow the caller out of it',
    ).toBeDefined()
  })

  test('PLUGIN-05.1: A personal record cannot re-admit what the workspace excluded', async ({
    api,
  }) => {
    await api.setPluginActivation(pluginId, { reach: 'personal', enabled: true })

    const here = await api.listEffectivePlugins(workspaceId)
    expect(here.find((p) => p.pluginId === pluginId)).toBeUndefined()

    await api.clearPluginActivation(pluginId, 'personal')
  })

  test('PLUGIN-05.2: Clearing the workspace record hands it back to the default', async ({
    api,
  }) => {
    await api.clearPluginActivation(pluginId, 'workspace', undefined, workspaceId)

    const records = await api.listPluginActivations(pluginId, workspaceId)
    expect(records.every((r) => r.reach === 'everyone')).toBe(true)

    const here = await api.listEffectivePlugins(workspaceId)
    expect(here.find((p) => p.pluginId === pluginId)).toBeDefined()
  })

  // --- TASK-01: Create task asking agent about the skill ---------------------

  test('TASK-01: Create task with plugin skill question', async ({ api }) => {
    const { taskId, task } = await createTaskWithMessage(
      api,
      ctx.repositoryId!,
      `Do you have access to a skill called "${PLUGIN_SKILL}"? ` +
        'Reply with EXACTLY this format on separate lines:\n' +
        'SKILL_CHECK: YES or NO\n' +
        'SKILL_NAME: the exact skill name if found, or NONE',
    )

    expect(task.status).toBe('open')
    ctx.taskId = taskId
    progress(`Task created: ${taskId}`)
  })

  // --- SANDBOX-01: Wait for sandbox ready -----------------------------------

  test('SANDBOX-01: Sandbox provisions with plugin', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready, status } = await waitForSandboxReady(api, ctx.taskId!)

    expect(ready).toBe(true)
    expect(status!.sandboxStatus).toBe('ready')
    progress('Sandbox ready (plugin should be cloned)')
  })

  // --- SANDBOX-01.1: Check for plugin clone event (diagnostic) --------------

  test('SANDBOX-01.1: Check for plugin clone event', async () => {
    // Give events time to arrive
    await new Promise((r) => setTimeout(r, 2000))

    const taskEvents = events().getEventsForTask(ctx.taskId!)
    const summary = events().getEventSummary(ctx.taskId!)
    progress(`SSE event summary: ${JSON.stringify(summary)}`)

    // Check for PluginsCloned event
    const cloneEvent = taskEvents.find(
      (e) => e.type === 'sandbox' && e.payload?.reason === 'PluginsCloned',
    )

    if (cloneEvent) {
      progress('PluginsCloned event found — clone succeeded')
    } else {
      console.warn(
        '[Plugin] WARNING: No PluginsCloned event — sandbox image may not support plugin cloning. ' +
          'Rebuild sandbox image with latest code: docker/build-sandbox.sh --local',
      )
    }

    // Log all sandbox events for diagnostics
    const sandboxEvents = taskEvents.filter((e) => e.type === 'sandbox')
    for (const e of sandboxEvents) {
      progress(`  sandbox.${e.subType || '*'} reason=${e.payload?.reason || '?'}`)
    }
  })

  // --- AGENT-01: Wait for agent to complete ---------------------------------

  test('AGENT-01: Agent completes with plugin access', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    const { completed } = await waitForAgentTurnComplete(events(), api, ctx.taskId!, 1)

    expect(completed).toBe(true)
    progress('Agent turn completed')
  })

  // --- AGENT-01.1: Verify agent confirms skill access -----------------------

  test('AGENT-01.1: Agent confirms access to plugin skill', async ({ api }) => {
    const { messages } = await api.getTaskMessages(ctx.taskId!)

    // Log all messages for debugging
    for (const m of messages) {
      const texts = (m.content ?? [])
        .filter((b) => b.type === 'text' && !!b.text)
        .map((b) => b.text)
      if (texts.length > 0) {
        progress(`${m.role}: ${texts.join(' ').slice(0, 300)}`)
      }
    }

    // Find assistant messages
    const assistantMessages = messages.filter((m) => m.role === 'assistant')
    expect(assistantMessages.length).toBeGreaterThanOrEqual(1)

    // Concatenate all assistant message text content
    const fullResponse = assistantMessages
      .flatMap((m) => m.content ?? [])
      .filter((block) => block.type === 'text' && !!block.text)
      .map((block) => block.text as string)
      .join('\n')

    // Check for PluginsCloned event — if missing, the failure is expected
    const taskEvents = events().getEventsForTask(ctx.taskId!)
    const hadCloneEvent = taskEvents.some(
      (e) => e.type === 'sandbox' && e.payload?.reason === 'PluginsCloned',
    )

    if (!hadCloneEvent) {
      console.warn(
        "[Plugin] No PluginsCloned SSE event — sandbox image likely doesn't support plugin cloning. " +
          'Rebuild with: docker/build-sandbox.sh --local',
      )
    }

    // Verify the agent found the skill
    expect(
      fullResponse,
      'Agent should confirm skill access (SKILL_CHECK: YES). ' +
        'If sandbox image is stale, rebuild it to include clone_plugins support.',
    ).toContain('SKILL_CHECK: YES')
    expect(fullResponse.toLowerCase()).toContain(PLUGIN_SKILL)
  })
})

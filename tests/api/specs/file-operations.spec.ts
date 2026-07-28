/**
 * Chain: File Operations (file-operations.md)
 *
 * Tests workspace file API: listing, reading, tree, search, and write.
 * Requires a sandbox in ready state but no agent execution.
 *
 * Chain:
 *   SANDBOX-01 (precondition) →
 *   FILE-01 → FILE-02 → FILE-03 → FILE-04 → FILE-05
 */

import { budget } from '../../shared/profile'
import { ChainContext } from '../context/chain-context'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { createTaskWithoutMessage, waitForSandboxReady } from '../steps'

/** The last segment of a workspace-relative path. */
function basename(path: string): string {
  return path.split('/').pop() ?? path
}

test.describe('Chain: File Operations', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id

    // Create task without message (agent stays idle)
    const { taskId, task } = await createTaskWithoutMessage(api, repo.id)
    ctx.taskId = taskId
    ctx.task = task

    progress(`Task created: ${taskId}`)
  })

  test.afterAll(async ({ api }) => {
    await discardTask(api, ctx.taskId)
  })

  // ─── Precondition: Sandbox ready ────────────────────────────────────

  test('SANDBOX-01: Sandbox provisions for file operations', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready, status } = await waitForSandboxReady(api, ctx.taskId!)

    expect(ready).toBe(true)
    expect(status).not.toBeNull()
    ctx.sandboxStatus = status!

    progress(`Sandbox ready: ${status?.sandboxStatus}`)
  })

  // ─── FILE-01: List workspace files ──────────────────────────────────

  test('FILE-01: List workspace files', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget())

    // Poll until files appear (workspace setup takes a few seconds)
    let files: string[] = []
    const deadline = Date.now() + 45_000

    while (Date.now() < deadline) {
      files = await api.listWorkspaceFiles(ctx.taskId!, '.', false)
      if (files.length > 0) {
        break
      }
      await new Promise((r) => setTimeout(r, 2000))
    }

    expect(files.length).toBeGreaterThan(0)

    // The browser is rooted at the workspace and repositories are mounted one
    // level below it, so what must be there is a checkout — not any particular
    // file inside it. Which repository the environment provides is its own
    // business, so the rest of the chain works from what is found here.
    const repoDir = files.find((path) => path.endsWith('/') && !path.startsWith('.'))
    expect(
      repoDir,
      `no repository is mounted under the workspace: ${files.join(', ')}`,
    ).toBeDefined()

    const inside = await api.listWorkspaceFiles(ctx.taskId!, repoDir!.replace(/\/$/, ''), true)
    const plain = inside.filter(
      (path) => !(path.endsWith('/') || path.split('/').some((part) => part.startsWith('.'))),
    )
    // Prefer something inside a subdirectory: the file a repository keeps at
    // its root is as often a symlink as a file (AGENTS.md pointing at
    // CLAUDE.md, say), and a symlink is not what the file APIs walk.
    const sample = plain.find((path) => path.split('/').length > 2) ?? plain[0]
    expect(sample, `the checkout at ${repoDir} has no files in it`).toBeDefined()

    ctx.set('repoDir', repoDir!)
    ctx.set('sampleFile', sample!)

    progress(`Listed ${files.length} entries; reading ${sample}`)
  })

  // ─── FILE-02: Read workspace file ───────────────────────────────────

  test('FILE-02: Read workspace file', async ({ api }) => {
    const sample = ctx.get<string>('sampleFile')!
    const content = await api.readWorkspaceFile(ctx.taskId!, sample)

    expect(content, `${sample} could not be read`).not.toBeNull()
    expect(content!.length).toBeGreaterThan(0)

    progress(`Read ${sample} (${content!.length} chars)`)
  })

  // ─── FILE-03: File tree with git status overlay ─────────────────────

  test('FILE-03: File tree with git status overlay', async ({ api }) => {
    const tree = await api.getFileTree(ctx.taskId!)

    expect(tree).toBeDefined()
    expect(tree.tree).toBeDefined()
    expect(tree.tree.length).toBeGreaterThan(0)

    progress(`File tree has ${tree.tree.length} root entries`)
  })

  // ─── FILE-04: Search files by name ──────────────────────────────────

  test('FILE-04: Search files by name', async ({ api }) => {
    const sample = ctx.get<string>('sampleFile')!
    const result = await api.searchFilesByName(ctx.taskId!, basename(sample))

    expect(result.files.length).toBeGreaterThan(0)

    // The file listing found it, so the search has to as well.
    const found = result.files.some((file) => file.path === sample)
    expect(found, `search did not return ${sample}`).toBe(true)

    progress(`Search found ${result.files.length} matches`)
  })

  // ─── FILE-05: Write file to workspace ───────────────────────────────

  test('FILE-05: Write file to workspace', async ({ api }) => {
    const testContent = `// E2E test file written at ${new Date().toISOString()}\nexport const hello = "world";\n`
    const path = `${ctx.get<string>('repoDir')}e2e-test-file.ts`

    await api.writeFile(ctx.taskId!, path, testContent)

    // Verify by reading back
    const content = await api.readWorkspaceFile(ctx.taskId!, path)

    expect(content).not.toBeNull()
    expect(content).toContain('E2E test file')
    expect(content).toContain('export const hello = "world"')

    progress('Write + read-back verified')
  })
})

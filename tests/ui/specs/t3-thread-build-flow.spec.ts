/**
 * Chain: build, preview, change and archive a thread through the T3 UI
 *
 * Starts a thread on the configured project from the browser and has the agent
 * build a page. The test, not the agent, serves it: it runs the project's dev
 * server Script from the thread panel and checks the page in the preview pane.
 * A second instruction changes the page, and the change must reach the Script
 * that is already running. Last, it archives the thread.
 *
 * `beforeAll` declares the Script on the project when it is missing, and it
 * stays there for later runs.
 *
 * Chain:
 *   T3UI-01 → T3UI-02 → T3UI-03 → T3UI-04 → T3UI-05 → T3UI-06
 */

import { randomBytes } from 'node:crypto'
import { expect, type FrameLocator, type Page, test } from '@playwright/test'
import { Backend } from '../helpers/backend'
import { storageStatePath, uiEnv } from '../helpers/env'

const env = uiEnv()
const TURN_TIMEOUT = 12 * 60_000
const PORT = 5173
const DEV_SERVER = {
  name: 'dev-server',
  label: 'Dev server',
  command: `bun run dev -- --host 0.0.0.0 --port ${PORT} --strictPort`,
  port: PORT,
}
const runId = randomBytes(3).toString('hex')
const firstHeading = `Moatable UI check ${runId}`
const secondHeading = `Moatable UI check ${runId} updated`

const buildInstruction = [
  `Build a single page in this repository. It shows an h1 reading exactly "${firstHeading}",`,
  'one short paragraph, a counter rendered as "Clicks: N" starting at 0, and a button labelled',
  '"Add one" that adds 1 to the counter per click.',
  'Do not start a dev server; it is started separately. Do not commit, push or open a PR.',
].join(' ')

const changeInstruction = [
  `Change the page: the h1 now reads exactly "${secondHeading}", and the button is labelled`,
  '"Add two" and adds 2 to the counter per click.',
  'Do not start a dev server; one is already running. Do not commit, push or open a PR.',
].join(' ')

test.describe('Chain: T3 thread build, preview, change and archive', () => {
  test.describe.configure({ mode: 'serial' })

  let page: Page
  let backend: Backend
  let taskId: string | undefined
  // One page for the whole chain, so the thread stays open between steps. A
  // page made here does not inherit `use` from the config, so it is given
  // the signed-in state and base URL explicitly.
  test.beforeAll(async ({ browser }) => {
    backend = await Backend.open(env)
    if (await backend.ensureWorkspaceScript(env.project, DEV_SERVER)) {
      console.log(`[ui setup] declared the ${DEV_SERVER.label} Script on ${env.project}`)
    }
    const context = await browser.newContext({
      baseURL: env.baseUrl,
      storageState: storageStatePath,
      viewport: { width: 1440, height: 900 },
    })
    page = await context.newPage()
  })

  test.afterEach(async ({}, testInfo) => {
    if (testInfo.status === testInfo.expectedStatus) return
    await testInfo.attach('page', { body: await page.screenshot(), contentType: 'image/png' })
  })

  test.afterAll(async () => {
    await backend.closeIfOpen(taskId)
    await backend.dispose()
    await page.context().close()
  })

  // ─── T3UI-01: Start a thread on the project from the UI ─────────────

  test('T3UI-01: Start a thread on the project and send the build instruction', async () => {
    await page.goto('/')
    await page.getByRole('button', { name: 'New thread', exact: true }).click()
    await page
      .getByRole('dialog', { name: 'Command palette' })
      .getByRole('option', { name: new RegExp(`^${escapeRegExp(env.project)} `) })
      .click()
    await expect(
      page.getByRole('heading', { name: `What should we build in ${env.project}?` }),
      `the draft did not open on project ${env.project}`,
    ).toBeVisible()

    const sentAt = new Date(Date.now() - 5000)
    const composer = page.getByRole('textbox', { name: 'Message' })
    await composer.fill(buildInstruction)
    await composer.press('Enter')

    taskId = await backend.findTaskBySentText(env.project, firstHeading, sentAt, 120_000)
    const threadPath = `/${await backend.environmentId()}/${taskId}`
    // A thread the client has not received yet redirects to a blank draft,
    // so the navigation is retried until the thread page renders.
    await expect(async () => {
      if (new URL(page.url()).pathname !== threadPath) await page.goto(threadPath)
      await expect(page.getByRole('button', { name: /^Thread actions for / })).toBeVisible({
        timeout: 10_000,
      })
    }, `the thread page for task ${taskId} never opened`).toPass({ timeout: 120_000 })
    test.info().annotations.push({ type: 'task', description: taskId! })
  })

  // ─── T3UI-02: The agent builds the page ─────────────────────────────

  test('T3UI-02: The agent finishes the build turn', async () => {
    test.setTimeout(TURN_TIMEOUT + 60_000)
    await backend.waitForTurns(taskId!, 1, TURN_TIMEOUT)
  })

  // ─── T3UI-03: Serve it from the UI and use it in the preview ────────

  test('T3UI-03: Run the dev server Script from the UI and use the page', async () => {
    // A page left open while the sandbox started keeps showing "Sandbox
    // required" in the preview pane and parks the iframe behind the chat,
    // where clicks land on the chat instead. Reloading picks up the sandbox.
    await page.reload()
    await page.getByRole('button', { name: `Run ${DEV_SERVER.label}` }).click()

    const preview = await previewShowing(page, firstHeading)
    for (let i = 0; i < 3; i++) await clickInPreview(page, preview, 'Add one')
    await expect(
      preview.getByText('Clicks: 3'),
      'three clicks on "Add one" did not reach 3',
    ).toBeVisible()
  })

  // ─── T3UI-04: A second instruction goes to the same thread ──────────

  test('T3UI-04: Send the change instruction to the running thread', async () => {
    test.setTimeout(TURN_TIMEOUT + 60_000)
    const composer = page.getByRole('textbox', { name: 'Message' })
    await composer.fill(changeInstruction)
    await composer.press('Enter')
    await backend.waitForTurns(taskId!, 2, TURN_TIMEOUT)
  })

  // ─── T3UI-05: The change reaches the Script that already runs ───────

  test('T3UI-05: The existing Script serves the changed page', async () => {
    const scripts = await backend.scripts(taskId!)
    const devServer = scripts.find((s) => s.name === DEV_SERVER.name)
    expect(
      devServer?.port,
      `the project lost its "${DEV_SERVER.name}" Script; the task holds ${JSON.stringify(scripts)}`,
    ).toBe(PORT)

    const preview = await previewShowing(page, secondHeading)
    await clickInPreview(page, preview, 'Add two')
    await clickInPreview(page, preview, 'Add two')
    await expect(
      preview.getByText('Clicks: 4'),
      'two clicks on "Add two" did not reach 4',
    ).toBeVisible()
  })

  // ─── T3UI-06: Archive the thread ────────────────────────────────────

  test('T3UI-06: Archive the thread from its actions menu', async () => {
    await page.getByRole('button', { name: /^Thread actions for / }).click()
    await page.getByRole('button', { name: 'Archive thread' }).click()
    // Only shown when the user's "Archive confirmation" setting is on.
    const confirm = page.getByRole('alertdialog').getByRole('button', { name: 'Confirm' })
    if (await confirm.isVisible({ timeout: 3000 }).catch(() => false)) await confirm.click()

    await expect
      .poll(async () => (await backend.task(taskId!)).status, {
        message: 'archiving in the UI did not close the task',
        timeout: 60_000,
      })
      .not.toBe('open')
  })
})

/**
 * Waits for the preview pane to show `heading`, reloading the frame between
 * tries: the first load can land before Vite listens, and an error page never
 * retries by itself.
 */
async function previewShowing(page: Page, heading: string): Promise<FrameLocator> {
  const frame = page.locator('iframe[title="Preview"]')
  await expect(frame, 'running the Script opened no preview pane').toBeVisible({ timeout: 60_000 })
  const preview = page.frameLocator('iframe[title="Preview"]')
  await expect(async () => {
    await frame.evaluate((el: HTMLIFrameElement) => el.setAttribute('src', el.src))
    await expect(preview.getByRole('heading', { level: 1 })).toHaveText(heading, {
      timeout: 5000,
    })
  }, `the preview never showed the heading "${heading}"`).toPass({ timeout: 120_000 })
  return preview
}

/**
 * Clicks a button inside the preview like a user would. When something in the
 * app covers it, fails with where the preview iframe sits and what covers the
 * button, instead of retrying until the test times out.
 */
async function clickInPreview(page: Page, preview: FrameLocator, name: string): Promise<void> {
  const button = preview.getByRole('button', { name })
  try {
    await button.click({ timeout: 15_000 })
  } catch (error) {
    const frame = await page.locator('iframe[title="Preview"]').boundingBox()
    const target = await button.boundingBox()
    const cover = target
      ? await page.evaluate(
          ({ x, y }) => {
            const e = document.elementFromPoint(x, y)
            return e ? `${e.tagName} ${String(e.className).slice(0, 80)}` : 'nothing'
          },
          { x: target.x + target.width / 2, y: target.y + target.height / 2 },
        )
      : 'no button box'
    throw new Error(
      `"${name}" in the preview cannot be clicked: iframe at ${JSON.stringify(frame)}, ` +
        `button at ${JSON.stringify(target)}, covered by ${cover}`,
      { cause: error },
    )
  }
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

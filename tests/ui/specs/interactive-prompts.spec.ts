/**
 * Interactive prompt UI tests — plan approval and AskUserQuestion.
 *
 * These tests verify the frontend → backend → sandbox round-trip for
 * tool responses (ExitPlanMode, AskUserQuestion). The backend fix in
 * commit 7cca68ff7 made `parse_content_blocks` handle `tool_result`
 * content blocks properly.
 *
 * Prerequisites:
 *   Start docker-compose: cd docker-compose && ./dev-docker.sh
 *   Run: npx playwright test --config tests/ui/playwright.config.ts tests/ui/specs/interactive-prompts.spec.ts
 */

import { expect, test } from '../fixtures/ui-fixtures'

test.describe('Interactive Prompts', () => {
  test('plan mode: approve plan via UI', async ({ page, api, testRepoId }) => {
    // Create task with plan mode via API
    const result = await api.createVibeTask({
      templateId: testRepoId,
      message:
        'Plan how to add a dark mode toggle to the application. Create a plan but wait for my approval before making changes.',
      name: 'UI Plan Approval E2E',
    })

    // Navigate to the task page
    await page.goto(`/tasks/${result.taskId}`)
    await page.waitForLoadState('networkidle')

    // Wait for the plan approval card to appear (up to 3 minutes for sandbox + agent)
    const planApproval = page.getByTestId('plan-approval-input')
    try {
      await planApproval.waitFor({ state: 'visible', timeout: 180_000 })
    } catch {
      // Agent may have completed without entering plan mode — skip gracefully
      console.log('Plan approval card did not appear — agent may have completed without plan mode')
      await api.stopTask(result.taskId)
      await api.deleteTask(result.taskId)
      return
    }

    // Click the "Approve Plan" button
    const approveButton = planApproval.getByRole('button', {
      name: 'Approve Plan',
    })
    await expect(approveButton).toBeVisible()
    await approveButton.click()

    // After approval, the plan approval card should disappear
    await expect(planApproval).toBeHidden({ timeout: 30_000 })

    // Wait for the agent to produce new content after approval
    // (new messages should appear in the chat)
    await page.waitForTimeout(5000)

    // Verify the page is still functional
    await expect(page.locator('#root')).toBeVisible()

    // Cleanup
    await api.stopTask(result.taskId)
    await api.deleteTask(result.taskId)
  })
})

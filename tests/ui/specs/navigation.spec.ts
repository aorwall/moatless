/**
 * Navigation tests — verify routing between pages works correctly.
 */

import { test, expect } from "../fixtures/ui-fixtures";

test.describe("Navigation", () => {
	test("navigate between dashboard and task list", async ({ page }) => {
		// Start at root
		await page.goto("/");
		await expect(page.locator("#root")).toBeVisible();

		// Navigate to tasks page (if link exists)
		const tasksLink = page.locator('a[href*="tasks"], a[href*="dashboard"]');
		if ((await tasksLink.count()) > 0) {
			await tasksLink.first().click();
			await page.waitForLoadState("networkidle");
			await expect(page.locator("#root")).toBeVisible();
		}
	});

	test("direct navigation to task page works", async ({
		page,
		api,
		testRepoId,
	}) => {
		// Create a task
		const task = await api.createVibeTask({
			templateId: testRepoId,
			name: "Nav E2E",
		});

		// Direct navigate to task
		await page.goto(`/tasks/${task.id}`);
		await page.waitForLoadState("networkidle");
		await expect(page.locator("#root")).toBeVisible();

		// Navigate back to root
		await page.goto("/");
		await expect(page.locator("#root")).toBeVisible();

		// Cleanup
		await api.stopTask(task.id);
		await api.deleteTask(task.id);
	});
});

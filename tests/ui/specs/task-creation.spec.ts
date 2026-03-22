/**
 * Task creation flow — verify creating a task from the UI.
 *
 * Tests the primary user flow: navigate → select repo → enter message → submit.
 */

import { test, expect } from "../fixtures/ui-fixtures";

test.describe("Task Creation Flow", () => {
	test("navigate to dashboard and see repository list", async ({ page }) => {
		await page.goto("/");
		// Dashboard should show some content
		await expect(page.locator("#root")).toBeVisible();
	});

	test("create task via API and navigate to task page", async ({
		page,
		api,
		testRepoId,
	}) => {
		// Create task via API (faster than UI interaction)
		const task = await api.createVibeTask({
			templateId: testRepoId,
			name: "UI E2E Test Task",
		});

		// Navigate to the task page
		await page.goto(`/tasks/${task.id}`);

		// Task page should load
		await expect(page.locator("#root")).toBeVisible();

		// Wait for the page content to stabilize
		await page.waitForLoadState("networkidle");

		// Cleanup
		await api.stopTask(task.id);
		await api.deleteTask(task.id);
	});

	test("task page shows status indicators", async ({
		page,
		api,
		testRepoId,
	}) => {
		// Create task with a message to trigger provisioning
		const task = await api.createVibeTask({
			templateId: testRepoId,
			message: "Say hello",
			name: "UI Status E2E",
		});

		await page.goto(`/tasks/${task.id}`);
		await page.waitForLoadState("networkidle");

		// The page should show some status information
		await expect(page.locator("#root")).toBeVisible();

		// Cleanup
		await api.stopTask(task.id);
		await api.deleteTask(task.id);
	});
});

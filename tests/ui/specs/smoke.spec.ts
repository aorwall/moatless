/**
 * Smoke tests — verify the application loads and core endpoints work.
 *
 * These are fast tests (~30s total) that validate basic app health.
 */

import { test, expect } from "../fixtures/ui-fixtures";

test.describe("Smoke Tests", () => {
	test("application loads and renders main layout", async ({ page }) => {
		await page.goto("/");
		// The app should render a root element
		await expect(page.locator("#root")).toBeVisible();
	});

	test("page has correct title", async ({ page }) => {
		await page.goto("/");
		const title = await page.title();
		expect(title).toMatch(/Moatless|Vibe/i);
	});

	test("repositories API endpoint is accessible", async ({ api }) => {
		const repos = await api.listRepositories();
		expect(Array.isArray(repos)).toBe(true);
	});

	test("SSE event stream endpoint is accessible", async ({ page }) => {
		const response = await page.request.get("/api/v1/events/stream", {
			timeout: 5000,
		});
		// SSE endpoint should return 200 with streaming content
		expect(response.status()).toBe(200);
	});
});

import { expect, test } from "@playwright/test";

/**
 * Browser-level guard for the query-client hazard: `handleAuthError` is wired
 * into both the query and mutation caches, so a wrong-password 401 would
 * otherwise trigger the global `/login` redirect. That both interrupts the form
 * and burns the 30-second redirect cooldown, breaking the *next* genuine
 * session expiry.
 *
 * These specs run signed-out, so they opt out of the suite's storageState.
 */
test.use({ storageState: { cookies: [], origins: [] } });

test.describe("login page", () => {
	test("wrong password shows an inline error and stays on /login", async ({
		page,
	}) => {
		await page.goto("/login");

		await page.getByLabel("Username").fill("admin");
		await page.getByLabel("Password").fill("definitely-the-wrong-password");
		await page.getByRole("button", { name: "Sign in" }).click();

		await expect(page.getByRole("alert")).toContainText(
			"Invalid username or password",
		);
		await expect(page).toHaveURL(/\/login/);

		// The redirect cooldown must be untouched — if it were stamped here, a
		// real session expiry within the next 30s would silently fail to redirect.
		const cooldown = await page.evaluate(() =>
			sessionStorage.getItem("auth_redirect_ts"),
		);
		expect(cooldown).toBeNull();
	});

	test("no redirect loop after a failed attempt", async ({ page }) => {
		await page.goto("/login");

		for (let attempt = 0; attempt < 2; attempt++) {
			await page.getByLabel("Username").fill("admin");
			await page.getByLabel("Password").fill(`wrong-${attempt}`);
			await page.getByRole("button", { name: "Sign in" }).click();
			await expect(page.getByRole("alert")).toBeVisible();
		}

		await expect(page).toHaveURL(/\/login/);
		// The form is still usable rather than having been navigated away.
		await expect(page.getByLabel("Username")).toBeVisible();
	});

	test("correct password signs in and leaves /login", async ({ page }) => {
		const username = process.env.ADMIN_USERNAME || "admin";
		const password = process.env.ADMIN_PASSWORD || "ci-admin-password-1234";

		await page.goto("/login");
		await page.getByLabel("Username").fill(username);
		await page.getByLabel("Password").fill(password);
		await page.getByRole("button", { name: "Sign in" }).click();

		await expect(page).not.toHaveURL(/\/login/);
	});
});

/**
 * Global teardown for API E2E tests.
 *
 * Runs once after all test files complete.
 * Chain afterAll blocks handle per-chain cleanup (e.g., deleting GH repos).
 */

export default async function globalTeardown(): Promise<void> {
	console.log("\n[global-teardown] API E2E tests complete.\n");
}

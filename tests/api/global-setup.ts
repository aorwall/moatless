/**
 * Global setup for API E2E tests.
 *
 * Runs once before all test files:
 * 1. Validates GITHUB_PAT is present (fail-fast)
 * 2. Validates gh CLI is available and authenticated
 * 3. Truncates all DB tables for a clean slate
 */

import { execSync } from "node:child_process";
import { resetDatabase } from "./helpers/db-reset";

export default async function globalSetup(): Promise<void> {
	console.log("\n[global-setup] Starting API E2E test setup...\n");

	// 1. Validate GITHUB_PAT
	if (!process.env.GITHUB_PAT) {
		throw new Error(
			"[global-setup] GITHUB_PAT environment variable is required.\n" +
			"  Set it to a GitHub Personal Access Token with repo scope.\n" +
			"  Usage: GITHUB_PAT=ghp_xxx bun run test:api",
		);
	}

	// 2. Validate gh CLI is available and authenticated
	try {
		const user = execSync("gh api user --jq .login", {
			encoding: "utf-8",
			env: { ...process.env, GH_TOKEN: process.env.GITHUB_PAT },
			timeout: 15_000,
		}).trim();
		console.log(`[global-setup] GitHub CLI authenticated as: ${user}`);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		throw new Error(
			`[global-setup] GitHub CLI validation failed: ${message}\n` +
			"  Make sure 'gh' is installed and GITHUB_PAT is valid.",
		);
	}

	// 3. Reset database
	await resetDatabase();

	console.log("[global-setup] Setup complete.\n");
}

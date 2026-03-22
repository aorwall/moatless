/**
 * Database reset helper for API E2E tests.
 *
 * Truncates all application tables to provide a clean slate for each test run.
 * Runs via `docker exec` against the running postgres container.
 */

import { execSync } from "node:child_process";

const POSTGRES_CONTAINER = "moatless-postgres";
const POSTGRES_USER = process.env.POSTGRES_USER || "moatless";
const POSTGRES_DB = process.env.POSTGRES_DB || "moatless";

/**
 * Tables to truncate, ordered to respect FK dependencies.
 * TRUNCATE ... CASCADE handles FK constraints automatically.
 */
const APP_TABLES = [
	"messages",
	"tasks",
	"repository_plugin_repos",
	"plugin_repos",
	"repositories",
	"channel_configs",
	"channel_threads",
	"team_members",
	"teams",
	"sessions",
	"api_keys",
	"provider_tokens",
	"users",
];

/**
 * Truncate all application tables in the docker-compose PostgreSQL instance.
 *
 * Uses TRUNCATE ... CASCADE to handle foreign key constraints.
 * Skips silently if tables don't exist (fresh database — migrations will create them).
 */
export async function resetDatabase(): Promise<void> {
	const tableList = APP_TABLES.join(", ");
	const sql = `TRUNCATE ${tableList} CASCADE;`;

	try {
		execSync(
			`docker exec ${POSTGRES_CONTAINER} psql -U ${POSTGRES_USER} -d ${POSTGRES_DB} -c "${sql}"`,
			{ stdio: "pipe", timeout: 10_000 },
		);
		console.log(`[global-setup] Database reset: truncated ${APP_TABLES.length} tables`);
	} catch (error) {
		// If the container isn't running or tables don't exist, warn but don't fail.
		// The backend creates tables on startup via migrations.
		const message = error instanceof Error ? error.message : String(error);
		if (message.includes("does not exist")) {
			console.warn("[global-setup] Some tables don't exist yet (migrations pending) — skipping truncate");
		} else {
			console.warn(`[global-setup] Database reset failed: ${message}`);
			console.warn("[global-setup] Make sure docker-compose is running: cd docker-compose && ./dev-docker.sh");
			throw error;
		}
	}
}

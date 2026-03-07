/**
 * Seeds test users in the docker-compose PostgreSQL instance.
 *
 * Uses `docker exec psql` (same pattern as db-reset.ts).
 */

import { execSync } from "node:child_process";

const POSTGRES_CONTAINER = "moatless-postgres";
const POSTGRES_USER = process.env.POSTGRES_USER || "moatless";
const POSTGRES_DB = process.env.POSTGRES_DB || "moatless";

export const ALICE = { userId: "alice-test", login: "alice" };
export const BOB = { userId: "bob-test", login: "bob" };
export const CAROL_ADMIN = { userId: "carol-admin", login: "carol", role: "admin" };

export async function seedTestUsers(): Promise<void> {
	const now = new Date().toISOString();
	const sql = [
		`INSERT INTO users (id, login, provider, role, created_at, last_login_at) VALUES ('${ALICE.userId}', '${ALICE.login}', 'test', 'user', '${now}', '${now}') ON CONFLICT (id) DO NOTHING;`,
		`INSERT INTO users (id, login, provider, role, created_at, last_login_at) VALUES ('${BOB.userId}', '${BOB.login}', 'test', 'user', '${now}', '${now}') ON CONFLICT (id) DO NOTHING;`,
		`INSERT INTO users (id, login, provider, role, created_at, last_login_at) VALUES ('${CAROL_ADMIN.userId}', '${CAROL_ADMIN.login}', 'test', 'admin', '${now}', '${now}') ON CONFLICT (id) DO NOTHING;`,
	].join(" ");

	execSync(
		`docker exec ${POSTGRES_CONTAINER} psql -U ${POSTGRES_USER} -d ${POSTGRES_DB} -c "${sql}"`,
		{ stdio: "pipe", timeout: 10_000 },
	);
}

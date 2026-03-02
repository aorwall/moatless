/**
 * Chain: Repository CRUD (repository.md)
 *
 * Tests repository creation from a real GitHub URL, retrieval, updates,
 * listing, and deletion.
 *
 * Chain:
 *   REPO-01 → REPO-01.1 → REPO-02 → REPO-03 → REPO-04 →
 *   REPO-05 → REPO-05.1
 *
 * Standalone: REPO-06, REPO-07
 *
 * Requires: GITHUB_PAT env var (validated in global-setup)
 */

import { randomUUID } from "node:crypto";
import { test, expect } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { createTestRepo, deleteTestRepo } from "../helpers/gh-repo";
import {
	createRepository,
	getRepository,
	updateRepository,
	listRepositories,
	deleteRepository,
	createRepositoryWithMissingFields,
	getNonExistentRepository,
} from "../steps";

// =============================================================================
// Chain: Repository CRUD
// =============================================================================

test.describe("Chain: Repository CRUD", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	let ghRepoFullName: string | undefined;

	test.beforeAll(async () => {
		// Create a private GitHub repo for this chain
		const { repoUrl, repoFullName } = await createTestRepo();
		ghRepoFullName = repoFullName;
		ctx.set("repoUrl", repoUrl);
	});

	test.afterAll(async ({ api }) => {
		// Delete the backend repository if it was created
		if (ctx.repositoryId) {
			await api.deleteRepository(ctx.repositoryId).catch(() => {});
		}
		// Delete the GitHub repo
		if (ghRepoFullName) {
			await deleteTestRepo(ghRepoFullName).catch(() => {});
		}
	});

	// ─── REPO-01: Create repository from GitHub URL ─────────────────────

	test("REPO-01: Create repository from GitHub URL", async ({ api }) => {
		const repoUrl = ctx.get<string>("repoUrl")!;

		const result = await createRepository(api, {
			name: "E2E Test Repository",
			remoteUrl: repoUrl,
			kind: "remote",
			provider: "github",
			description: "Created by API E2E test",
		});

		expect(result.repositoryId).toBeTruthy();
		expect(result.repository.name).toBe("E2E Test Repository");
		expect(result.repository.kind).toBe("remote");
		expect(result.repository.remoteUrl).toBe(repoUrl);

		ctx.repositoryId = result.repositoryId;
		ctx.repository = result.repository;
	});

	// ─── REPO-01.1: Repository appears in listing ───────────────────────

	test("REPO-01.1: Repository appears in listing", async ({ api }) => {
		const repos = await listRepositories(api);

		const found = repos.some((r) => r.id === ctx.repositoryId);
		expect(found).toBe(true);
	});

	// ─── REPO-02: Get repository by ID ──────────────────────────────────

	test("REPO-02: Get repository by ID", async ({ api }) => {
		const repo = await getRepository(api, ctx.repositoryId!);

		expect(repo.id).toBe(ctx.repositoryId);
		expect(repo.name).toBeTruthy();
		expect(repo.kind).toBe("remote");
		expect(repo.remoteUrl).toBeTruthy();
		expect(repo.createdAt).toBeTruthy();
	});

	// ─── REPO-03: Update repository metadata ────────────────────────────

	test("REPO-03: Update repository metadata", async ({ api }) => {
		const updated = await updateRepository(api, ctx.repositoryId!, {
			name: "Updated E2E Repository",
			description: "Updated by API E2E test",
		});

		expect(updated.name).toBe("Updated E2E Repository");
		expect(updated.description).toBe("Updated by API E2E test");

		// Confirm via GET
		const fetched = await getRepository(api, ctx.repositoryId!);
		expect(fetched.name).toBe("Updated E2E Repository");
		expect(fetched.description).toBe("Updated by API E2E test");
	});

	// ─── REPO-04: List repositories ─────────────────────────────────────

	test("REPO-04: List repositories", async ({ api }) => {
		const repos = await listRepositories(api);

		expect(repos.length).toBeGreaterThan(0);

		const found = repos.find((r) => r.id === ctx.repositoryId);
		expect(found).toBeDefined();
		expect(found!.name).toBe("Updated E2E Repository");
	});

	// ─── REPO-05: Delete repository ─────────────────────────────────────

	test("REPO-05: Delete repository", async ({ api }) => {
		const response = await deleteRepository(api, ctx.repositoryId!);

		expect(response.status).toBeLessThan(300);
	});

	// ─── REPO-05.1: Repository removed from listing ─────────────────────

	test("REPO-05.1: Repository removed from listing", async ({ api }) => {
		const repos = await listRepositories(api);

		const found = repos.some((r) => r.id === ctx.repositoryId);
		expect(found).toBe(false);

		// Prevent afterAll from trying to delete again
		ctx.repositoryId = undefined;
	});
});

// =============================================================================
// Standalone: Error Cases
// =============================================================================

// ─── REPO-06: Create repository with missing required fields (422) ──────

test("REPO-06: Create repository with missing required fields", async ({ api }) => {
	const response = await createRepositoryWithMissingFields(api);

	expect(response.status).toBeGreaterThanOrEqual(400);
	expect(response.status).toBeLessThan(500);
});

// ─── REPO-07: Get non-existent repository (404) ─────────────────────────

test("REPO-07: Get non-existent repository", async ({ api }) => {
	const fakeId = randomUUID();
	const response = await getNonExistentRepository(api, fakeId);

	expect(response.status).toBe(404);
});

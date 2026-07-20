/**
 * Chain: Access Control (02-access-control-scenarios.md)
 *
 * Tests scope-based visibility for repositories and tasks across
 * global, team, and user scopes.
 *
 * Chain 1: AUTH-20 → AUTH-20.1 → AUTH-21 → AUTH-21.1 → AUTH-21.2 →
 *          AUTH-22 → AUTH-22.1 → AUTH-22.2
 * Chain 2: AUTH-23 → AUTH-23.1 → AUTH-23.2 → AUTH-23.3
 * Chain 3: AUTH-24 → AUTH-24.1
 * Standalone: AUTH-25, AUTH-26
 */

import type { ApiClient } from "../../shared/api-client";
import { ChainContext } from "../context/chain-context";
import { expect, loginAs, test } from "../fixtures/auth-fixtures";
import { seedTestUsers } from "../helpers/seed-users";

/**
 * Helper to create a repository with scope fields.
 * Uses `createRepositoryWithScope` which supports scope/teamId fields
 * not present in the generated `CreateRepositoryRequest` type.
 */
async function createScopedRepo(
	client: ApiClient,
	config: {
		name: string;
		kind: string;
		scope: string;
		teamId?: string;
		remoteUrl?: string;
	},
) {
	return client.createRepositoryWithScope(config);
}

// =============================================================================
// Chain 1: Repository Scope Filtering
// =============================================================================

test.describe("Chain 1: Repository Scope Filtering", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	const suffix = Date.now().toString(36);
	const repoIds: string[] = [];

	test.beforeAll(async () => {
		await seedTestUsers();
	});

	test.afterAll(async ({ alice }) => {
		for (const id of repoIds) {
			await alice.deleteRepository(id).catch(() => {});
		}
		const teamId = ctx.get<string>("teamId");
		if (teamId) {
			await alice.deleteTeam(teamId).catch(() => {});
		}
	});

	// Setup: Alice creates a team and adds Bob
	test("Setup: Alice creates team and adds Bob", async ({ alice, bob }) => {
		const team = await alice.createTeam({
			name: "Scope Team",
			slug: `scope-${suffix}`,
		});
		ctx.set("teamId", team.id);
		await alice.addTeamMember(team.id, await bob.userId(), "member");
	});

	// ─── AUTH-20: Create global-scoped repository ────────────────────────

	test("AUTH-20: Create global-scoped repository", async ({ alice }) => {
		const repo = await createScopedRepo(alice, {
			name: `Global Repo ${suffix}`,
			kind: "template",
			scope: "global",
		});

		expect(repo.id).toBeTruthy();
		expect(repo.scope).toBe("global");

		ctx.set("globalRepoId", repo.id);
		repoIds.push(repo.id);
	});

	// ─── AUTH-20.1: Other user can see global repository ─────────────────

	test("AUTH-20.1: Other user can see global repository", async ({ bob }) => {
		const bobWithTeam = await loginAs("bob");
		const repos = await bobWithTeam.listRepositories();
		const found = repos.some((r) => r.id === ctx.get<string>("globalRepoId"));
		expect(found).toBe(true);
	});

	// ─── AUTH-21: Create team-scoped repository ──────────────────────────

	test("AUTH-21: Create team-scoped repository", async ({ alice }) => {
		const teamId = ctx.get<string>("teamId")!;
		const aliceWithTeam = await loginAs("alice");

		const repo = await createScopedRepo(aliceWithTeam, {
			name: `Team Repo ${suffix}`,
			kind: "template",
			scope: "team",
			teamId,
		});

		expect(repo.id).toBeTruthy();
		expect(repo.scope).toBe("team");
		expect(repo.teamId).toBe(teamId);

		ctx.set("teamRepoId", repo.id);
		repoIds.push(repo.id);
	});

	// ─── AUTH-21.1: Team member can see team repository ──────────────────

	test("AUTH-21.1: Team member can see team repository", async ({ bob }) => {
		const bobWithTeam = await loginAs("bob");
		const repos = await bobWithTeam.listRepositories();
		const found = repos.some((r) => r.id === ctx.get<string>("teamRepoId"));
		expect(found).toBe(true);
	});

	// ─── AUTH-21.2: Non-member cannot see team repository ────────────────

	test("AUTH-21.2: Non-member cannot see team repository", async ({ api }) => {
		// Carol without team membership (non-admin view)
		const outsider = await loginAs("carol-nonadmin");
		const repos = await outsider.listRepositories();
		const found = repos.some((r) => r.id === ctx.get<string>("teamRepoId"));
		expect(found).toBe(false);
	});

	// ─── AUTH-22: Create user-scoped repository ──────────────────────────

	test("AUTH-22: Create user-scoped repository", async ({ alice }) => {
		const repo = await createScopedRepo(alice, {
			name: `User Repo ${suffix}`,
			kind: "template",
			scope: "user",
		});

		expect(repo.id).toBeTruthy();
		expect(repo.scope).toBe("user");

		ctx.set("userRepoId", repo.id);
		repoIds.push(repo.id);
	});

	// ─── AUTH-22.1: Creator can see user repository ──────────────────────

	test("AUTH-22.1: Creator can see user repository", async ({ alice }) => {
		const repos = await alice.listRepositories();
		const found = repos.some((r) => r.id === ctx.get<string>("userRepoId"));
		expect(found).toBe(true);
	});

	// ─── AUTH-22.2: Other user cannot see user repository ────────────────

	test("AUTH-22.2: Other user cannot see user repository", async ({ bob }) => {
		const repos = await bob.listRepositories();
		const found = repos.some((r) => r.id === ctx.get<string>("userRepoId"));
		expect(found).toBe(false);
	});
});

// =============================================================================
// Chain 2: Task Scope Inheritance
// =============================================================================

test.describe("Chain 2: Task Scope Inheritance", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	const suffix = Date.now().toString(36);

	test.beforeAll(async () => {
		await seedTestUsers();
	});

	test.afterAll(async ({ alice }) => {
		if (ctx.taskId) {
			await alice.deleteTask(ctx.taskId).catch(() => {});
		}
		if (ctx.repositoryId) {
			await alice.deleteRepository(ctx.repositoryId).catch(() => {});
		}
		const teamId = ctx.get<string>("teamId");
		if (teamId) {
			await alice.deleteTeam(teamId).catch(() => {});
		}
	});

	// Setup: Alice creates a team, adds Bob, and creates a team-scoped repo
	test("Setup: Alice creates team + team-scoped repo", async ({ alice, bob }) => {
		const team = await alice.createTeam({
			name: "Task Scope Team",
			slug: `task-scope-${suffix}`,
		});
		ctx.set("teamId", team.id);

		await alice.addTeamMember(team.id, await bob.userId(), "member");

		const aliceWithTeam = await loginAs("alice");

		const repo = await createScopedRepo(aliceWithTeam, {
			name: `Task Scope Repo ${suffix}`,
			kind: "template",
			scope: "team",
			teamId: team.id,
		});
		ctx.repositoryId = repo.id;
	});

	// ─── AUTH-23: Create task under team-scoped repository ───────────────

	test("AUTH-23: Create task under team-scoped repository", async ({ alice }) => {
		const aliceWithTeam = await loginAs("alice");

		const task = await aliceWithTeam.createVibeTask({
			templateId: ctx.repositoryId!,
			message: "Test task for scope inheritance",
			name: `Scoped Task ${suffix}`,
		});

		expect(task.taskId).toBeTruthy();
		ctx.taskId = task.taskId;
		ctx.task = task;
	});

	// ─── AUTH-23.1: Task inherits team scope from repository ─────────────

	test("AUTH-23.1: Task inherits team scope from repository", async ({ alice }) => {
		const aliceWithTeam = await loginAs("alice");
		const task = await aliceWithTeam.getTask(ctx.taskId!);

		// The task response should reflect the scope inherited from the repo
		expect((task as any).scope).toBe("team");
		expect((task as any).teamId).toBe(ctx.get<string>("teamId"));
	});

	// ─── AUTH-23.2: Team member can see the task ─────────────────────────

	test("AUTH-23.2: Team member can see the task", async ({ bob }) => {
		const bobWithTeam = await loginAs("bob");
		const tasks = await bobWithTeam.listTasks();
		const found = tasks.items.some((t) => t.taskId === ctx.taskId);
		expect(found).toBe(true);
	});

	// ─── AUTH-23.3: Non-member cannot see the task ───────────────────────

	test("AUTH-23.3: Non-member cannot see the task", async ({ api }) => {
		const outsider = await loginAs("outsider");
		const tasks = await outsider.listTasks();
		const found = tasks.items.some((t) => t.taskId === ctx.taskId);
		expect(found).toBe(false);
	});
});

// =============================================================================
// Chain 3: Admin Bypass
// =============================================================================

test.describe("Chain 3: Admin Bypass", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	const suffix = Date.now().toString(36);
	const repoIds: string[] = [];

	test.beforeAll(async () => {
		await seedTestUsers();
	});

	test.afterAll(async ({ alice }) => {
		for (const id of repoIds) {
			await alice.deleteRepository(id).catch(() => {});
		}
		const teamId = ctx.get<string>("teamId");
		if (teamId) {
			await alice.deleteTeam(teamId).catch(() => {});
		}
	});

	// Setup: Alice creates scoped resources
	test("Setup: Alice creates user-scoped and team-scoped repos", async ({
		alice,
	}) => {
		// Create a team (carol is NOT a member)
		const team = await alice.createTeam({
			name: "Admin Bypass Team",
			slug: `admin-bypass-${suffix}`,
		});
		ctx.set("teamId", team.id);

		// User-scoped repo (owned by alice)
		const userRepo = await createScopedRepo(alice, {
			name: `Alice Private ${suffix}`,
			kind: "template",
			scope: "user",
		});
		ctx.set("userRepoId", userRepo.id);
		repoIds.push(userRepo.id);

		// Team-scoped repo
		const aliceWithTeam = await loginAs("alice");
		const teamRepo = await createScopedRepo(aliceWithTeam, {
			name: `Team Only ${suffix}`,
			kind: "template",
			scope: "team",
			teamId: team.id,
		});
		ctx.set("teamRepoId", teamRepo.id);
		repoIds.push(teamRepo.id);
	});

	// ─── AUTH-24: Admin can see all scoped resources ─────────────────────

	test("AUTH-24: Admin can see all scoped resources", async ({ carol }) => {
		const repos = await carol.listRepositories();

		const hasUserRepo = repos.some(
			(r) => r.id === ctx.get<string>("userRepoId"),
		);
		const hasTeamRepo = repos.some(
			(r) => r.id === ctx.get<string>("teamRepoId"),
		);

		expect(hasUserRepo).toBe(true);
		expect(hasTeamRepo).toBe(true);
	});

	// ─── AUTH-24.1: Admin can fetch user-scoped repository by ID ─────────

	test("AUTH-24.1: Admin can fetch user-scoped repository by ID", async ({
		carol,
	}) => {
		const repo = await carol.getRepository(
			ctx.get<string>("userRepoId")!,
		);
		expect(repo.id).toBe(ctx.get<string>("userRepoId"));
		expect(repo.name).toContain("Alice Private");
	});
});

// =============================================================================
// Standalone
// =============================================================================

test.describe("Standalone: Access Control Filtering", () => {
	test.beforeAll(async () => {
		await seedTestUsers();
	});

// ─── AUTH-25: Listing repositories filters by scope ──────────────────────

test("AUTH-25: Listing repositories filters by scope", async ({
	alice,
	bob,
	api,
}) => {
	const suffix = Date.now().toString(36);

	// Create a team, add alice as member
	const team = await alice.createTeam({
		name: "Filter Team",
		slug: `filter-${suffix}`,
	});

	const aliceWithTeam = await loginAs("alice");

	// Create repos with different scopes
	const globalRepo = await createScopedRepo(aliceWithTeam, {
		name: `Global ${suffix}`,
		kind: "template",
		scope: "global",
	});
	const teamRepo = await createScopedRepo(aliceWithTeam, {
		name: `Team ${suffix}`,
		kind: "template",
		scope: "team",
		teamId: team.id,
	});
	const userRepo = await createScopedRepo(alice, {
		name: `User ${suffix}`,
		kind: "template",
		scope: "user",
	});

	try {
		// Bob is NOT a team member — should see global, not team or user repos
		const bobRepos = await bob.listRepositories();
		expect(bobRepos.some((r) => r.id === globalRepo.id)).toBe(true);
		expect(bobRepos.some((r) => r.id === teamRepo.id)).toBe(false);
		expect(bobRepos.some((r) => r.id === userRepo.id)).toBe(false);

		// Bob WITH team membership — should also see team repo
		const bobWithTeam = await loginAs("bob");
		const bobTeamRepos = await bobWithTeam.listRepositories();
		expect(bobTeamRepos.some((r) => r.id === globalRepo.id)).toBe(true);
		expect(bobTeamRepos.some((r) => r.id === teamRepo.id)).toBe(true);
		expect(bobTeamRepos.some((r) => r.id === userRepo.id)).toBe(false);
	} finally {
		await alice.deleteRepository(globalRepo.id).catch(() => {});
		await alice.deleteRepository(teamRepo.id).catch(() => {});
		await alice.deleteRepository(userRepo.id).catch(() => {});
		await alice.deleteTeam(team.id).catch(() => {});
	}
});

// ─── AUTH-26: Listing tasks filters by scope ─────────────────────────────

test("AUTH-26: Listing tasks filters by scope", async ({ alice, api }) => {
	const suffix = Date.now().toString(36);

	// Create a team
	const team = await alice.createTeam({
		name: "Task Filter Team",
		slug: `task-filter-${suffix}`,
	});

	const aliceWithTeam = await loginAs("alice");

	// Create a team-scoped repo + task
	const teamRepo = await createScopedRepo(aliceWithTeam, {
		name: `Task Filter Repo ${suffix}`,
		kind: "template",
		scope: "team",
		teamId: team.id,
	});

	const task = await aliceWithTeam.createVibeTask({
		templateId: teamRepo.id,
		message: "Scoped task test",
		name: `Scoped Task ${suffix}`,
	});

	try {
		// Outsider should NOT see the task
		const outsider = await loginAs("outsider2");
		const outsiderTasks = await outsider.listTasks();
		expect(outsiderTasks.items.some((t) => t.taskId === task.taskId)).toBe(false);

		// Team member should see the task
		const memberTasks = await aliceWithTeam.listTasks();
		expect(memberTasks.items.some((t) => t.taskId === task.taskId)).toBe(true);
	} finally {
		await alice.deleteTask(task.taskId).catch(() => {});
		await alice.deleteRepository(teamRepo.id).catch(() => {});
		await alice.deleteTeam(team.id).catch(() => {});
	}
});

}); // end Standalone describe

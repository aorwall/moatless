/**
 * Chain: Team CRUD, Membership, Credentials (01-team-scenarios.md)
 *
 * Tests team creation, retrieval, updates, deletion, membership management,
 * and credential storage.
 *
 * Chain 1: AUTH-01 → AUTH-01.1 → AUTH-02 → AUTH-03 → AUTH-04 → AUTH-04.1
 * Chain 2: AUTH-05 → AUTH-05.1 → AUTH-07 → AUTH-07.1
 * Chain 3: AUTH-08 → AUTH-08.1 → AUTH-08.2 → AUTH-08.2.1
 * Standalone: AUTH-09, AUTH-10
 */

import { test, expect } from "../fixtures/auth-fixtures";
import { ChainContext } from "../context/chain-context";
import { seedTestUsers } from "../helpers/seed-users";

// =============================================================================
// Chain 1: Team CRUD
// =============================================================================

test.describe("Chain 1: Team CRUD", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	const suffix = Date.now().toString(36);

	test.beforeAll(async () => {
		await seedTestUsers();
	});

	test.afterAll(async ({ alice }) => {
		const teamId = ctx.get<string>("teamId");
		if (teamId) {
			await alice.deleteTeam(teamId).catch(() => {});
		}
	});

	// ─── AUTH-01: Create a team ──────────────────────────────────────────

	test("AUTH-01: Create a team", async ({ alice }) => {
		const team = await alice.createTeam({
			name: "Engineering",
			slug: `engineering-${suffix}`,
		});

		expect(team.id).toBeTruthy();
		expect(team.name).toBe("Engineering");
		expect(team.slug).toBe(`engineering-${suffix}`);
		expect(team.createdAt).toBeTruthy();
		expect(team.updatedAt).toBeTruthy();

		ctx.set("teamId", team.id);
		ctx.set("teamSlug", team.slug);
	});

	// ─── AUTH-01.1: Team appears in listing ──────────────────────────────

	test("AUTH-01.1: Team appears in listing", async ({ alice }) => {
		const teams = await alice.listTeams();
		const found = teams.some((t) => t.id === ctx.get<string>("teamId"));
		expect(found).toBe(true);
	});

	// ─── AUTH-02: Get team by ID ─────────────────────────────────────────

	test("AUTH-02: Get team by ID", async ({ alice }) => {
		const team = await alice.getTeam(ctx.get<string>("teamId")!);

		expect(team.name).toBe("Engineering");
		expect(team.slug).toBe(`engineering-${suffix}`);
	});

	// ─── AUTH-03: Update team ────────────────────────────────────────────

	test("AUTH-03: Update team", async ({ alice }) => {
		const updated = await alice.updateTeam(ctx.get<string>("teamId")!, {
			name: "Platform Engineering",
			description: "Platform team",
		});

		expect(updated.name).toBe("Platform Engineering");
		expect(updated.description).toBe("Platform team");
		expect(updated.slug).toBe(`engineering-${suffix}`);
	});

	// ─── AUTH-04: Delete team ────────────────────────────────────────────

	test("AUTH-04: Delete team", async ({ alice }) => {
		await alice.deleteTeam(ctx.get<string>("teamId")!);
	});

	// ─── AUTH-04.1: Team removed from listing ────────────────────────────

	test("AUTH-04.1: Team removed from listing", async ({ alice }) => {
		const teams = await alice.listTeams();
		const found = teams.some((t) => t.id === ctx.get<string>("teamId"));
		expect(found).toBe(false);

		// Prevent afterAll from trying to delete again
		ctx.set("teamId", undefined);
	});
});

// =============================================================================
// Chain 2: Team Membership
// =============================================================================

test.describe("Chain 2: Team Membership", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	const suffix = Date.now().toString(36);

	test.beforeAll(async () => {
		await seedTestUsers();
	});

	test.afterAll(async ({ alice }) => {
		const teamId = ctx.get<string>("teamId");
		if (teamId) {
			await alice.deleteTeam(teamId).catch(() => {});
		}
	});

	// Setup: Alice creates a team
	test("Setup: Alice creates team for membership tests", async ({ alice }) => {
		const team = await alice.createTeam({
			name: "Membership Team",
			slug: `membership-${suffix}`,
		});
		ctx.set("teamId", team.id);
	});

	// ─── AUTH-05: Add member to team ─────────────────────────────────────

	test("AUTH-05: Add member to team", async ({ alice, bob }) => {
		// Server-assigned id — logins are not user ids.
		ctx.set("bobId", await bob.userId());
		await alice.addTeamMember(
			ctx.get<string>("teamId")!,
			ctx.get<string>("bobId")!,
			"member",
		);
	});

	// ─── AUTH-05.1: Member appears in team member listing ────────────────

	test("AUTH-05.1: Member appears in team member listing", async ({ alice }) => {
		const members = await alice.listTeamMembers(ctx.get<string>("teamId")!);
		const membership = members.find(
			(m) => m.userId === ctx.get<string>("bobId"),
		);

		expect(membership).toBeDefined();
		expect(membership!.role).toBe("member");
	});

	// ─── AUTH-07: Remove member from team ────────────────────────────────

	test("AUTH-07: Remove member from team", async ({ alice }) => {
		await alice.removeTeamMember(
			ctx.get<string>("teamId")!,
			ctx.get<string>("bobId")!,
		);
	});

	// ─── AUTH-07.1: Removed member no longer in listing ──────────────────

	test("AUTH-07.1: Removed member no longer in listing", async ({ alice }) => {
		const members = await alice.listTeamMembers(ctx.get<string>("teamId")!);
		const membership = members.find(
			(m) => m.userId === ctx.get<string>("bobId"),
		);
		expect(membership).toBeUndefined();
	});
});

// =============================================================================
// Chain 3: Team Credentials
// =============================================================================

test.describe("Chain 3: Team Credentials", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	const suffix = Date.now().toString(36);

	test.beforeAll(async () => {
		await seedTestUsers();
	});

	test.afterAll(async ({ alice }) => {
		const teamId = ctx.get<string>("teamId");
		if (teamId) {
			await alice.deleteTeam(teamId).catch(() => {});
		}
	});

	// Setup: Alice creates a team
	test("Setup: Alice creates team for credential tests", async ({ alice }) => {
		const team = await alice.createTeam({
			name: "Credentials Team",
			slug: `credentials-${suffix}`,
		});
		ctx.set("teamId", team.id);
	});

	// ─── AUTH-08: Upsert team credential ─────────────────────────────────

	test("AUTH-08: Upsert team credential", async ({ alice }) => {
		await alice.upsertTeamCredential(
			ctx.get<string>("teamId")!,
			"github",
			{ accessToken: "ghp_test_token_12345" },
		);
	});

	// ─── AUTH-08.1: Credential appears in listing ────────────────────────

	test("AUTH-08.1: Credential appears in listing", async ({ alice }) => {
		const creds = await alice.listTeamCredentials(
			ctx.get<string>("teamId")!,
		);
		const github = creds.find((c) => c.provider === "github");

		expect(github).toBeDefined();
		// Access token must NOT be exposed
		expect(JSON.stringify(github)).not.toContain("ghp_test_token");
	});

	// ─── AUTH-08.2: Delete team credential ───────────────────────────────

	test("AUTH-08.2: Delete team credential", async ({ alice }) => {
		await alice.deleteTeamCredential(
			ctx.get<string>("teamId")!,
			"github",
		);
	});

	// ─── AUTH-08.2.1: Credential removed from listing ────────────────────

	test("AUTH-08.2.1: Credential removed from listing", async ({ alice }) => {
		const creds = await alice.listTeamCredentials(
			ctx.get<string>("teamId")!,
		);
		const github = creds.find((c) => c.provider === "github");
		expect(github).toBeUndefined();
	});
});

// =============================================================================
// Standalone
// =============================================================================

test.describe("Standalone: Team Error Cases", () => {
	test.beforeAll(async () => {
		await seedTestUsers();
	});

	// ─── AUTH-09: Duplicate slug → 409 ───────────────────────────────────

	test("AUTH-09: Create team with duplicate slug fails", async ({ alice }) => {
		const suffix = Date.now().toString(36);
		const slug = `dup-slug-${suffix}`;

		// Create first team
		const team = await alice.createTeam({ name: "First", slug });

		try {
			// Attempt duplicate
			const response = await alice.createTeamRaw({ name: "Second", slug });
			expect(response.status).toBe(409);
		} finally {
			await alice.deleteTeam(team.id).catch(() => {});
		}
	});

	// ─── AUTH-10: Non-member credential upsert → 403 ─────────────────────

	test("AUTH-10: Non-member cannot manage team credentials", async ({
		alice,
		bob,
	}) => {
		const suffix = Date.now().toString(36);

		// Alice creates a team (bob is not a member)
		const team = await alice.createTeam({
			name: "Private Team",
			slug: `private-${suffix}`,
		});

		try {
			const response = await bob.upsertTeamCredentialRaw(team.id, "github", {
				accessToken: "ghp_unauthorized",
			});
			expect(response.status).toBe(403);
		} finally {
			await alice.deleteTeam(team.id).catch(() => {});
		}
	});
});

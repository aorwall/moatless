/**
 * Standalone: Sandbox Error Cases (sandbox-operations.md)
 *
 * Independent tests — each creates and tears down its own resources.
 *
 * Scenarios:
 *   SANDBOX-06: Sandbox provision failure (invalid image)
 *   SANDBOX-07: Sandbox crash loop detected early
 *   SERVER-02: Server readiness timeout detected as failure
 */

import { execSync } from "node:child_process";
import { test, expect } from "../fixtures/api-fixtures";
import { waitForSandboxReady, waitForServerFailed } from "../steps";

// ─── SANDBOX-06: Sandbox provision failure (invalid image) ──────────────

test("SANDBOX-06: Sandbox provision failure (invalid image)", async ({ api }, testInfo) => {
	testInfo.setTimeout(120_000);

	const repo = await api.createRepository({
		name: `crash-test-imagepull-${Date.now()}`,
		kind: "template",
		remoteUrl: "https://github.com/aorwall/hello-world-ui.git",
		dockerImage: "nonexistent-image-12345:latest",
	});

	let taskId: string | undefined;
	try {
		const task = await api.createVibeTask({ templateId: repo.id });
		taskId = task.taskId;

		const { ready } = await waitForSandboxReady(api, taskId, 60_000);
		expect(ready).toBe(false);

		// In Docker mode, the provisioning error occurs at container creation time
		// (image pull failure). The status resolver sees no container → not_created.
		// The error was reported via the provision response / sandbox.error SSE event,
		// not persisted in the status endpoint.
		const status = await api.getWorkspaceStatus(taskId);
		expect(status.environmentStatus).not.toBe("ready");
		console.log(
			`[SANDBOX-06] Status: ${status.environmentStatus}, error: ${status.environmentError ?? "none (provision-time error)"}`,
		);
	} finally {
		if (taskId) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
		await api.deleteRepository(repo.id).catch(() => {});
	}
});

// ─── SANDBOX-07: Sandbox crash loop detected early ──────────────────────

test("SANDBOX-07: Sandbox crash loop detected early", async ({ api }, testInfo) => {
	testInfo.setTimeout(180_000);

	// Build the crashing image — skip test if Docker build fails
	try {
		execSync(
			"docker build -t moatless-sandbox-crash:test -f docker/Dockerfile.sandbox-crash docker/",
			{ encoding: "utf-8", timeout: 60_000, stdio: "pipe" },
		);
	} catch {
		test.skip(true, "Docker build not available — skipping crash loop test");
		return;
	}

	const repo = await api.createRepository({
		name: `crash-test-crashloop-${Date.now()}`,
		kind: "template",
		remoteUrl: "https://github.com/aorwall/hello-world-ui.git",
		dockerImage: "moatless-sandbox-crash:test",
	});

	let taskId: string | undefined;
	try {
		const startTime = Date.now();
		const task = await api.createVibeTask({ templateId: repo.id });
		taskId = task.taskId;

		// Wait for sandbox — should fail fast once crash loop is detected.
		// The backend detects crash loops when restart_count >= max_crash_restart_count (default: 5).
		// With Docker's exponential backoff (~100ms doubling), 5 restarts accumulate in ~3s,
		// so detection should happen well within 120s.
		const { ready } = await waitForSandboxReady(api, taskId, 120_000);
		const elapsed = Date.now() - startTime;

		expect(ready).toBe(false);

		const status = await api.getWorkspaceStatus(taskId);
		// Crash loop should produce an environmentError
		expect(status.environmentError).toBeTruthy();
		// Should detect crash loop much faster than the full timeout
		expect(elapsed).toBeLessThan(120_000);

		console.log(`[SANDBOX-07] Error: ${status.environmentError} (detected in ${elapsed}ms)`);
	} finally {
		if (taskId) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
		await api.deleteRepository(repo.id).catch(() => {});
	}
});

// ─── SERVER-02: Server readiness timeout detected as failure ─────────

test("SERVER-02: Server readiness timeout detected as failure", async ({ api }, testInfo) => {
	testInfo.setTimeout(240_000);

	// Create a repository with a server that intentionally doesn't bind the
	// expected port. The start command runs a process that stays alive but
	// never listens, so the K8s readiness probe stays unhealthy.
	const repo = await api.createRepository({
		name: `server-timeout-test-${Date.now()}`,
		kind: "template",
		remoteUrl: "https://github.com/aorwall/hello-world-ui.git",
		servers: [
			{
				name: "preview",
				label: "Preview",
				port: 5173,
				dockerImage: "aorwall/moatless-vibe-sandbox:latest",
				startCommands: ["sleep 300"], // alive but never binds port
				startupTimeoutSeconds: 30, // short timeout for faster test
				default: true,
			},
		],
	});

	let taskId: string | undefined;
	try {
		const task = await api.createVibeTask({ templateId: repo.id });
		taskId = task.taskId;

		// Wait for sandbox environment to be ready first
		const { ready } = await waitForSandboxReady(api, taskId, 180_000);
		expect(ready).toBe(true);

		// Now wait for the server — it should eventually report "failed"
		// after the 30s startup timeout elapses
		const server = await waitForServerFailed(api, taskId, 120_000);

		expect(server).not.toBeNull();
		expect(server!.status).toBe("failed");
		expect(server!.error).toBeTruthy();

		console.log(
			`[SERVER-02] Server status: ${server!.status}, error: ${server!.error}`,
		);
	} finally {
		if (taskId) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
		await api.deleteRepository(repo.id).catch(() => {});
	}
});

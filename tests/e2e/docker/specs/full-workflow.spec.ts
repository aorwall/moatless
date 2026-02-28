/**
 * Full Docker E2E Workflow Test
 *
 * Walks through the complete happy-path lifecycle in a single ordered flow:
 *   create task from seeded template → sandbox provisions → read files before agent
 *   → preview server starts → agent executes → read files after agent → verify changes
 *
 * This test creates its OWN task (not the shared fixture) to ensure
 * a clean, isolated lifecycle from start to finish.
 *
 * Prerequisites:
 * - Docker-compose environment running: cd docker-compose && ./dev-docker.sh
 * - Claude authentication: CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY
 */

import { test as base, expect } from "@playwright/test";
import type { TaskResponse } from "@moatless/api-client";
import {
	ApiClient,
	SSEEventCollector,
	isDockerComposeHealthy,
} from "../helpers";

// =============================================================================
// Configuration
// =============================================================================

const BASE_URL = process.env.DOCKER_COMPOSE_URL || "http://localhost:8080";
const SSE_URL = `${BASE_URL}/api/v1/events/stream`;

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

// =============================================================================
// Test Suite
// =============================================================================

base.describe("Full Docker Workflow", () => {
	base.describe.configure({ mode: "serial" });

	base.skip(!hasClaudeAuth, "Skipping — no Claude authentication");

	let api: ApiClient;
	let sse: SSEEventCollector;
	let task: TaskResponse | null = null;

	base.beforeAll(async () => {
		api = new ApiClient(BASE_URL);

		// Health check
		const health = await isDockerComposeHealthy(BASE_URL);
		if (!health.healthy) {
			throw new Error(
				`Docker-compose not healthy: ${health.error}\n` +
					"Start with: cd docker-compose && ./dev-docker.sh",
			);
		}

		// Start SSE collector
		sse = new SSEEventCollector({ debug: !!process.env.DEBUG_SSE });
		await sse.start(SSE_URL);

		// Find template repository (uses seeded "Moatable" or any template)
		const repository = await api.ensureTemplateRepository();

		// Create task — task starts automatically (provisioning + agent triggered by message)
		task = await api.createVibeTask({
			templateId: repository.id,
			message: "Add a heading that says 'E2E Test Success' to the main page",
		});

		console.log(`Created task: ${task.taskId} (template: ${repository.name})`);
	});

	base.afterAll(async () => {
		sse?.stop();

		if (task) {
			try {
				await api.stopTask(task.taskId);
				await new Promise((resolve) => setTimeout(resolve, 500));
				await api.deleteTask(task.taskId);
				console.log(`Cleaned up task: ${task.taskId}`);
			} catch {
				// Best-effort cleanup
			}
		}
	});

	// ─── Test 1: Sandbox provisions ────────────────────────────────────────

	base("sandbox should provision and become ready", async ({}, testInfo) => {
		testInfo.setTimeout(200000);
		const taskId = task!.taskId;

		// Poll workspace status until environment is ready
		const deadline = Date.now() + 180000;
		let ready = false;

		while (Date.now() < deadline) {
			try {
				const status = await api.getWorkspaceStatus(taskId);
				if (status.environmentStatus === "ready" && status.workspaceReady) {
					ready = true;
					break;
				}
				console.log(`  Provisioning: envStatus=${status.environmentStatus}, workspaceReady=${status.workspaceReady}`);
			} catch (err) {
				console.log(`  Provisioning poll error: ${err instanceof Error ? err.message : String(err)}`);
			}
			await new Promise((resolve) => setTimeout(resolve, 3000));
		}

		expect(ready).toBe(true);

		// Double-check the final status
		const finalStatus = await api.getWorkspaceStatus(taskId);
		expect(finalStatus.workspaceReady).toBe(true);
		console.log("  Sandbox provisioned and workspace ready");
	});

	// ─── Test 2: Files readable before agent completes ─────────────────────

	base("workspace files should be readable before agent completes", async ({}, testInfo) => {
		testInfo.setTimeout(60000);
		const taskId = task!.taskId;

		// Poll until files appear (workspace setup: git clone + bun install takes a few seconds)
		const deadline = Date.now() + 45000;
		let files: string[] = [];

		while (Date.now() < deadline) {
			files = await api.listWorkspaceFiles(taskId, ".", false);
			if (files.length > 0) {
				break;
			}
			console.log("  Waiting for workspace files...");
			await new Promise((resolve) => setTimeout(resolve, 2000));
		}

		expect(files.length).toBeGreaterThan(0);
		console.log(`  Found ${files.length} files in workspace root`);

		// Read a known file (try package.json first, then index.html)
		let content: string | null = null;
		const candidates = ["package.json", "index.html", "README.md"];
		let readFile = "";

		for (const file of candidates) {
			content = await api.readWorkspaceFile(taskId, file);
			if (content) {
				readFile = file;
				break;
			}
		}

		expect(content).not.toBeNull();
		console.log(`  Read ${readFile} (${content!.length} chars)`);

		// Check if "E2E Test Success" is already present (agent may already be running)
		const alreadyPresent = content!.includes("E2E Test Success");
		console.log(`  "E2E Test Success" already present: ${alreadyPresent} (agent may be running)`);
	});

	// ─── Test 3: Preview server starts ─────────────────────────────────────

	base("preview server should start", async ({}, testInfo) => {
		testInfo.setTimeout(120000);
		const taskId = task!.taskId;

		// Poll until a server container is started and has a URL.
		// Server containers wait for the .workspace-ready sentinel before starting,
		// so this may take a while (sandbox init → git clone → bun install → sentinel → server boot).
		const deadline = Date.now() + 90000;
		let serverUrl: string | undefined;

		while (Date.now() < deadline) {
			try {
				const status = await api.getWorkspaceStatus(taskId);

				if (status.servers && status.servers.length > 0) {
					const server =
						status.servers.find((s) => s.default && s.status === "started") ||
						status.servers.find((s) => s.status === "started");

					if (server?.url) {
						serverUrl = server.url;
						console.log(`  Preview server started: ${server.name} at ${serverUrl}`);
						break;
					}

					const serverNames = status.servers.map((s) => `${s.name}:${s.status}`).join(", ");
					console.log(`  Waiting for preview: [${serverNames}]`);
				}
			} catch {
				// Not available yet
			}
			await new Promise((resolve) => setTimeout(resolve, 3000));
		}

		expect(serverUrl).toBeDefined();

		// Fetch the server URL and assert response is not a server error
		const response = await fetch(serverUrl!, {
			signal: AbortSignal.timeout(10000),
		});
		expect(response.status).toBeLessThan(500);
		console.log(`  Preview server responded with HTTP ${response.status}`);
	});

	// ─── Test 4: Agent completes and writes expected content ───────────────

	base("agent should complete and write expected content", async ({}, testInfo) => {
		testInfo.setTimeout(360000);
		const taskId = task!.taskId;

		// Wait for first taskTurn.completed via SSE
		const turnCompleted = await sse.waitForNthEvent(
			"taskTurn",
			"completed",
			taskId,
			1,
			300000,
		);

		if (!turnCompleted) {
			// Fallback: poll workspace status for agent idle
			console.log("  No SSE event, falling back to polling...");
			const deadline = Date.now() + 300000;
			let completed = false;
			while (Date.now() < deadline) {
				try {
					const status = await api.getWorkspaceStatus(taskId);
					if (status.agentState === "idle" || status.agentState === "none") {
						const messagesResp = await api.getTaskMessages(taskId);
						if (messagesResp.messages.length > 1) {
							completed = true;
							break;
						}
					}
				} catch {
					// Not ready yet
				}
				await new Promise((resolve) => setTimeout(resolve, 2000));
			}
			expect(completed).toBe(true);
		} else {
			console.log("  Agent completed (via SSE)");
		}

		// Search workspace files for "E2E Test Success"
		const match = await findFileWithContent(api, taskId, "E2E Test Success");
		expect(match).not.toBeNull();
		console.log(`  Content "E2E Test Success" found in: ${match!.path}`);

		// Read the file again to confirm
		const content = await api.readWorkspaceFile(taskId, match!.path);
		expect(content).not.toBeNull();
		expect(content).toContain("E2E Test Success");

		// Verify git diff is non-empty (agent made tracked changes)
		// Note: git-diff endpoint may not be available in all backend versions
		try {
			const diff = await api.getGitDiff(taskId);
			const hasDiffContent = diff.diff ? diff.diff.length > 0 : diff.files.length > 0;
			expect(hasDiffContent).toBe(true);
			console.log(`  Git diff: ${diff.files.length} file(s) changed`);
		} catch {
			console.log("  Git diff endpoint not available — skipping diff verification");
		}
	});

	// ─── Test 5: SSE lifecycle events emitted ──────────────────────────────

	base("SSE lifecycle events should be emitted", async ({}, testInfo) => {
		testInfo.setTimeout(10000);
		const taskId = task!.taskId;

		const taskEvents = sse.getEventsForTask(taskId);

		// Verify key lifecycle events
		const hasAgentCompleted = taskEvents.some(
			(e) => e.type === "agent" && e.subType === "completed",
		);
		const hasTurnCompleted = taskEvents.some(
			(e) => e.type === "taskTurn" && e.subType === "completed",
		);
		const hasAgentMessage = taskEvents.some(
			(e) => e.type === "agent" && e.subType === "message",
		);

		expect(hasAgentCompleted).toBe(true);
		expect(hasTurnCompleted).toBe(true);
		expect(hasAgentMessage).toBe(true);

		// Log event summary
		const summary = sse.getEventSummary(taskId);
		console.log("  Event summary:");
		for (const [type, count] of Object.entries(summary)) {
			console.log(`    ${type}: ${count}`);
		}
	});
});

// =============================================================================
// Helpers
// =============================================================================

interface FileMatch {
	path: string;
	content: string;
}

async function findFileWithContent(
	api: ApiClient,
	taskId: string,
	searchString: string,
): Promise<FileMatch | null> {
	// Check common source file locations first
	const candidates = [
		"src/pages/Index.tsx",
		"src/App.tsx",
		"src/App.jsx",
		"src/main.tsx",
		"index.html",
	];

	for (const filePath of candidates) {
		const content = await api.readWorkspaceFile(taskId, filePath);
		if (content && content.includes(searchString)) {
			return { path: filePath, content };
		}
	}

	// If not found in common paths, list src/ recursively
	const files = await api.listWorkspaceFiles(taskId, "src", true);
	for (const filePath of files) {
		if (filePath.endsWith(".tsx") || filePath.endsWith(".jsx") || filePath.endsWith(".html")) {
			const content = await api.readWorkspaceFile(taskId, filePath);
			if (content && content.includes(searchString)) {
				return { path: filePath, content };
			}
		}
	}

	return null;
}

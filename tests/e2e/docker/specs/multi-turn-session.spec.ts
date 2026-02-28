/**
 * Multi-Turn Session Resume E2E Test
 *
 * Tests that the agent can handle multiple turns:
 * - Turn 1: Create content
 * - Turn 2: Send follow-up message to modify the content
 * - Verify each turn completes and modifies files correctly
 *
 * This test creates its OWN task (not the shared fixture) to ensure
 * clean state for multi-turn verification.
 *
 * Prerequisites:
 * - Docker-compose environment running
 * - Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY)
 */

import { test as base, expect } from "@playwright/test";
import type { TaskResponse } from "@moatless/api-client";
import {
	ApiClient,
	SSEEventCollector,
	isDockerComposeHealthy,
} from "../helpers";

// =============================================================================
// Test Setup
// =============================================================================

const BASE_URL = process.env.DOCKER_COMPOSE_URL || "http://localhost:8080";
const SSE_URL = `${BASE_URL}/api/v1/events/stream`;

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

// =============================================================================
// Test
// =============================================================================

base.describe("Multi-Turn Session Resume", () => {
	base.skip(!hasClaudeAuth, "Skipping — no Claude authentication");

	let api: ApiClient;
	let sse: SSEEventCollector;
	let task: TaskResponse | null = null;

	base.beforeAll(async () => {
		api = new ApiClient(BASE_URL);

		// Verify backend is healthy
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

	base("should handle two-turn conversation with content modification", async ({}, testInfo) => {
		testInfo.setTimeout(600000);

		// Get or create template repository
		const repository = await api.ensureTemplateRepository();

		// ── Turn 1: Create task with initial prompt ──────────────────────
		task = await api.createVibeTask({
			templateId: repository.id,
			message: "Add a heading that says 'E2E Test Success' to the main page",
		});

		const taskId = task.taskId;
		console.log(`[Turn 1] Task created: ${taskId}`);

		// Wait for first taskTurn.completed
		const turn1Completed = await sse.waitForNthEvent(
			"taskTurn",
			"completed",
			taskId,
			1,
			300000,
		);

		if (!turn1Completed) {
			console.log("[Turn 1] No SSE event, falling back to polling...");
			const idle = await waitForAgentIdle(api, taskId, 300000);
			expect(idle).toBe(true);
			console.log("[Turn 1] Agent completed (via polling)");
		} else {
			console.log("[Turn 1] Agent completed (via SSE)");
		}

		// Verify turn 1 content
		const turn1File = await findFileWithContent(api, taskId, "E2E Test Success");
		expect(turn1File).not.toBeNull();
		console.log(`[Turn 1] Content verified in: ${turn1File!.path}`);

		// ── Turn 2: Send follow-up message ───────────────────────────────
		console.log("[Turn 2] Sending follow-up message...");
		await api.sendMessage(
			taskId,
			"Change the 'E2E Test Success' heading to 'Multi-Turn Success' instead",
		);
		console.log("[Turn 2] Message sent");

		// Wait for SECOND taskTurn.completed
		const turn2Completed = await sse.waitForNthEvent(
			"taskTurn",
			"completed",
			taskId,
			2,
			300000,
		);

		if (!turn2Completed) {
			console.log("[Turn 2] No SSE event, falling back to polling...");
			const idle = await waitForAgentIdle(api, taskId, 300000);
			expect(idle).toBe(true);
			console.log("[Turn 2] Agent completed (via polling)");
		} else {
			console.log("[Turn 2] Agent completed (via SSE)");
		}

		// Verify turn 2 content — should have the modified heading
		const turn2File = await findFileWithContent(api, taskId, "Multi-Turn Success");
		expect(turn2File).not.toBeNull();
		console.log(`[Turn 2] Content verified in: ${turn2File!.path}`);

		// Verify event counts — at least 2 agent completions and 2 turn completions
		const agentCompletedCount = sse.countEvents("agent", "completed", taskId);
		const turnCompletedCount = sse.countEvents("taskTurn", "completed", taskId);

		expect(agentCompletedCount).toBeGreaterThanOrEqual(2);
		expect(turnCompletedCount).toBeGreaterThanOrEqual(2);

		console.log(`Event counts: agent.completed=${agentCompletedCount}, taskTurn.completed=${turnCompletedCount}`);

		// Log event summary
		const summary = sse.getEventSummary(taskId);
		console.log("Event summary:", summary);
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

async function waitForAgentIdle(
	api: ApiClient,
	taskId: string,
	timeoutMs: number,
): Promise<boolean> {
	const deadline = Date.now() + timeoutMs;
	let agentStarted = false;

	while (Date.now() < deadline) {
		try {
			const status = await api.getWorkspaceStatus(taskId);

			if (status.agentState === "running") {
				agentStarted = true;
				await new Promise((resolve) => setTimeout(resolve, 2000));
				continue;
			}

			if (status.agentState === "none" || status.agentState === "idle") {
				if (agentStarted) {
					return true;
				}
				const messagesResp = await api.getTaskMessages(taskId);
				if (messagesResp.messages.length > 1) {
					return true;
				}
			}
		} catch {
			// Transient error
		}
		await new Promise((resolve) => setTimeout(resolve, 2000));
	}
	return false;
}

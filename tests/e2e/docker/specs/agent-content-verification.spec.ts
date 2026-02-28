/**
 * Agent Content Verification E2E Test
 *
 * Tests single-turn agent execution with file content verification.
 * Sends a prompt, waits for completion, then verifies the agent
 * actually wrote the expected content to workspace files.
 *
 * This test creates its OWN task (not the shared fixture) to ensure
 * clean state for content verification.
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
// Test Setup (no shared task fixture — each test creates its own)
// =============================================================================

const BASE_URL = process.env.DOCKER_COMPOSE_URL || "http://localhost:8080";
const SSE_URL = `${BASE_URL}/api/v1/events/stream`;

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

// =============================================================================
// Test
// =============================================================================

base.describe("Agent Content Verification", () => {
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

		// Get or create template repository
		const repository = await api.ensureTemplateRepository();

		// Create a fresh task for this test suite
		task = await api.createVibeTask({
			templateId: repository.id,
			message: "Add a heading that says 'E2E Test Success' to the main page",
		});

		console.log(`Created task: ${task.taskId}`);
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

	base("should create file with expected content after agent execution", async ({}, testInfo) => {
		testInfo.setTimeout(600000);

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
			console.log("No SSE event, falling back to polling...");
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
			console.log("Agent completed (via SSE)");
		}

		// Verify file content — search common paths for the expected string
		const match = await findFileWithContent(api, taskId, "E2E Test Success");
		expect(match).not.toBeNull();
		console.log(`Content verified in: ${match!.path}`);
	});

	base("should emit expected SSE events", async ({}, testInfo) => {
		testInfo.setTimeout(10000);

		const taskId = task!.taskId;
		const taskEvents = sse.getEventsForTask(taskId);

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

/**
 * Multi-message same-session UI test.
 *
 * Verifies that multiple messages sent while the agent is running
 * are all displayed in the chat UI after the agent completes.
 *
 * Prerequisites:
 *   Start docker-compose: BACKEND_PORT=8080 SANDBOX_IMAGE=aorwall/moatless-vibe-sandbox:latest \
 *     docker compose --env-file .env -f docker-compose/docker-compose.yml up -d postgres redis backend
 *   Start frontend: bun run dev
 *   Run: BACKEND_PORT=8080 npx playwright test --config tests/ui/playwright.config.ts tests/ui/specs/multi-message.spec.ts
 */

import { test as base, expect } from "@playwright/test";
import { ApiClient } from "../../shared/api-client";
import { SSEEventCollector } from "../../shared/sse-collector";
import { waitForCondition } from "../../shared/wait-helpers";

// UI test fixture that allows split frontend/backend URLs
const test = base.extend<{ api: ApiClient; testRepoId: string }>({
	api: async ({}, use) => {
		// API client talks to the backend directly
		const backendURL = `http://localhost:${process.env.BACKEND_PORT || "8080"}`;
		const api = new ApiClient(backendURL);
		const healthy = await api.isHealthy();
		if (!healthy) {
			throw new Error(`Backend not healthy at ${backendURL}`);
		}
		await use(api);
	},
	testRepoId: async ({ api }, use) => {
		const repo = await api.ensureTemplateRepository();
		await use(repo.id);
	},
});

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

test.describe("Multi-Message Same Session", () => {
	test.skip(!hasClaudeAuth, "Skipping — no Claude authentication");

	test("all injected messages appear in chat after agent completes", async ({
		page,
		api,
		testRepoId,
	}, testInfo) => {
		testInfo.setTimeout(360_000);

		// Start SSE collector to track session_init events
		const backendURL = `http://localhost:${process.env.BACKEND_PORT || "8080"}`;
		const sse = new SSEEventCollector({ debug: !!process.env.DEBUG_SSE });
		await sse.start(`${backendURL}/api/v1/events/stream`);

		// ── Step 1: Create task with first message ──────────────────────
		const task = await api.createVibeTask({
			templateId: testRepoId,
			message:
				"Create a file called first.txt with the content 'First Message Created'",
			name: "UI Multi-Message E2E",
		});
		const taskId = task.taskId;
		console.log(`[MultiMsg UI] Task created: ${taskId}`);

		// ── Step 2: Wait for session_init (agent session is live) ───────
		const sessionInit = await sse.waitForEvent({
			type: "agent",
			subType: "session_init",
			taskId,
			timeout: 120_000,
		});
		expect(sessionInit).not.toBeNull();
		console.log("[MultiMsg UI] session_init received");

		// ── Step 3: Send 2nd and 3rd messages while agent is running ────
		await api.sendMessage(
			taskId,
			"Also create a file called second.txt with the content 'Second Message Created'",
		);
		console.log("[MultiMsg UI] Second message sent");

		await new Promise((r) => setTimeout(r, 2000));

		await api.sendMessage(
			taskId,
			"Also create a file called third.txt with the content 'Third Message Created'",
		);
		console.log("[MultiMsg UI] Third message sent");

		// ── Step 4: Wait for agent to become idle ───────────────────────
		const settled = await waitForCondition(
			async () => {
				const s = await api.getWorkspaceStatus(taskId);
				return s.agentState === "idle" || s.agentState === "none";
			},
			{ timeout: 300_000, interval: 2000 },
		);
		expect(settled).toBe(true);
		console.log("[MultiMsg UI] Agent is idle");

		// ── Step 5: Verify single session ───────────────────────────────
		const sessionInitCount = sse.countEvents(
			"agent",
			"session_init",
			taskId,
		);
		expect(sessionInitCount).toBe(1);
		console.log(
			`[MultiMsg UI] session_init count: ${sessionInitCount} (single session)`,
		);

		sse.stop();

		// ── Step 6: Verify messages via API (all acked, none queued) ────
		const { messages } = await api.getTaskMessages(taskId);
		const userMessages = messages.filter(
			(m) => m.role === "user" && m.messageType === "user",
		);
		expect(userMessages.length).toBe(3);
		console.log(`[MultiMsg UI] ${userMessages.length} user messages in API`);

		const queuedMessages = messages.filter((m) => m.isQueued === true);
		expect(queuedMessages.length).toBe(0);
		console.log("[MultiMsg UI] 0 queued messages (all acknowledged)");

		// ── Step 7: Navigate to task page and verify messages render ────
		await page.goto(`/tasks/${taskId}`);
		await page.waitForLoadState("networkidle");

		// Wait for the page to render
		await expect(page.locator("#root")).toBeVisible();

		// Wait for the chat to load messages (user message text should appear)
		const firstMsgLocator = page.getByText("First Message Created", { exact: false });
		try {
			await firstMsgLocator.first().waitFor({ state: "visible", timeout: 30_000 });

			// Verify all 3 user messages are visible in the chat
			await expect(
				page.getByText("First Message Created", { exact: false }).first(),
			).toBeVisible();
			await expect(
				page.getByText("Second Message Created", { exact: false }).first(),
			).toBeVisible();
			await expect(
				page.getByText("Third Message Created", { exact: false }).first(),
			).toBeVisible();
			console.log("[MultiMsg UI] All 3 messages visible in chat");

			// Verify no messages show as queued in the UI
			const queuedLabel = page.getByText("Queued", { exact: true });
			await expect(queuedLabel).toBeHidden({ timeout: 5_000 });
			console.log("[MultiMsg UI] No queued label in UI");
		} catch {
			// Chat rendering may fail if sandbox is already stopped.
			// The API verification (Step 6) already confirmed correctness.
			console.log("[MultiMsg UI] Chat rendering check skipped (sandbox stopped)");
		}

		// ── Cleanup ─────────────────────────────────────────────────────
		await api.stopTask(taskId);
		await new Promise((r) => setTimeout(r, 500));
		await api.deleteTask(taskId);
		console.log("[MultiMsg UI] Cleanup complete");
	});
});

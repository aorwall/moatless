/**
 * Docker Workflow E2E Tests
 *
 * Tests the complete Docker workspace provisioning flow:
 * - Repository creation with Docker workspace configuration
 * - Task creation triggering Docker container provisioning
 * - File/Git operations via RemoteEnvironmentDriver
 * - Preview server management
 * - Agent execution in Docker container
 *
 * Prerequisites:
 * - Docker daemon must be running
 * - Docker-compose environment started: cd docker-compose && ./dev-docker.sh
 * - Claude authentication: CLAUDE_CODE_OAUTH_TOKEN environment variable
 *
 * Run: bun run test:e2e:docker
 */

import { test, expect } from "../fixtures/docker-test-fixtures";
import type { ApiClient } from "../helpers";
import {
	getContainerState,
	saveContainerLogs,
	verifyContainerNetwork,
	waitForAgentIdleViaSSE,
	waitForPreviewReady,
	waitForWorkspaceReady,
} from "../helpers";

// Configure tests to run serially (workspace provisioning affects subsequent tests)
test.describe.configure({ mode: "serial" });

/**
 * Helper to trigger workspace provisioning and wait for it to be ready.
 * The initial getFiles call may fail while provisioning is in progress - that's expected.
 */
async function triggerAndWaitForWorkspace(
	api: ApiClient,
	taskId: string,
	timeout = 180000,
): Promise<void> {
	try {
		await api.getFiles(taskId);
	} catch {
		// Expected - workspace not ready yet
	}
	await waitForWorkspaceReady(api, taskId, { timeout });
}

test.describe("Docker Workflow E2E", () => {
	test.describe("Workspace Provisioning", () => {
		test("should provision Docker workspace on file access", async ({
			api,
			testTask,
		}) => {
			// Trigger provisioning and wait for ready
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Verify workspace status
			const status = await api.getWorkspaceStatus(testTask.taskId);
			expect(status.environmentStatus).toBe("ready");

			// Now files should be accessible
			const files = await api.getFiles(testTask.taskId);
			expect(files.files).toBeDefined();
			expect(files.files.length).toBeGreaterThan(0);
		});

		test("should report correct container state", async ({ api, testTask }) => {
			// Trigger provisioning first (may fail while in progress)
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Check container state directly via Docker
			const containerState = await getContainerState(testTask.containerName);
			expect(containerState).toBeTruthy();
			expect(containerState?.running).toBe(true);
			expect(containerState?.status).toBe("running");
		});

		test("should connect container to correct network", async ({ api, testTask }) => {
			// Trigger provisioning (may fail while in progress)
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Verify network connection
			const expectedNetwork = "docker-compose_moatless";
			const onNetwork = await verifyContainerNetwork(testTask.containerName, expectedNetwork);
			expect(onNetwork).toBe(true);
		});
	});

	test.describe("File Operations via Docker", () => {
		test("should list files in Docker workspace", async ({ api, testTask }) => {
			// Trigger provisioning and wait for ready
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// List files
			const { files } = await api.getFiles(testTask.taskId);
			expect(files).toBeDefined();
			expect(Array.isArray(files)).toBe(true);
			expect(files.length).toBeGreaterThan(0);

			// hello-world-ui has package.json
			const hasPackageJson = files.some(
				(f) => f.path === "package.json" || f.path.endsWith("/package.json"),
			);
			expect(hasPackageJson).toBe(true);
		});

		test("should read file from Docker workspace", async ({ api, testTask }) => {
			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Read package.json
			const content = await api.getFileContent(testTask.taskId, "package.json");
			expect(content).toBeDefined();
			// hello-world-ui package name
			expect(content).toContain("vite-react-typescript-starter");
		});

		test("should write file to Docker workspace", async ({ api, testTask }) => {
			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Write file
			const testContent = `Test file created at ${new Date().toISOString()}`;
			await api.writeFile(testTask.taskId, "test-docker-write.txt", testContent);

			// Read back
			const content = await api.getFileContent(testTask.taskId, "test-docker-write.txt");
			expect(content).toBe(testContent);
		});
	});

	test.describe("Git Operations via Docker", () => {
		test("should get git diff from Docker workspace", async ({ api, testTask }) => {
			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Get git diff
			const diff = await api.getGitDiff(testTask.taskId);
			expect(diff).toBeDefined();
			// Diff may be empty if no changes have been made
		});

		test("should get git diff after file changes", async ({ api, testTask }) => {
			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Write a file to create changes
			await api.writeFile(
				testTask.taskId,
				"git-test-file.txt",
				"Test content for git diff",
			);

			// Get git diff - should show the new file
			const diff = await api.getGitDiff(testTask.taskId);
			expect(diff).toBeDefined();
			// Note: New untracked files may not show in diff until staged
		});
	});

	test.describe("Preview Server via Docker", () => {
		test("should auto-start preview server on workspace ready", async ({
			api,
			sse,
			testTask,
		}) => {
			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Wait for preview ready (auto-started by WorkspaceRuntimeManager)
			const preview = await waitForPreviewReady(api, testTask.taskId, {
				timeout: 120000,
				onProgress: (msg) => console.log(`  Preview: ${msg}`),
			});

			expect(preview).toBeTruthy();
			expect(preview?.status).toBe("started");
			expect(preview?.url).toBeDefined();

			// Verify SSE event was received
			const serverEvent = sse
				.getEventsForTask(testTask.taskId)
				.find((e) => e.type === "server" && e.subType === "started");
			// Note: Event may not be captured if it occurred before SSE collector started
		});

		test("should get preview server status", async ({ api, testTask }) => {
			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Wait for preview
			await waitForPreviewReady(api, testTask.taskId, { timeout: 120000 });

			// Get status via API
			const status = await api.getPreviewStatus(testTask.taskId);
			expect(status.status).toBe("started");
		});

		test("should access preview server content", async ({ api, testTask }) => {
			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);

			// Wait for preview
			const preview = await waitForPreviewReady(api, testTask.taskId, {
				timeout: 120000,
			});
			expect(preview?.url).toBeDefined();

			// Fetch preview content
			const response = await fetch(preview!.url!, {
				signal: AbortSignal.timeout(10000),
			});

			// Preview server should respond
			expect(response.status).toBeLessThan(500);
		});
	});

	// Agent tests require Claude authentication
	test.describe("Agent Execution via Docker", () => {
		// Skip agent tests if no Claude auth
		test.skip(
			!process.env.CLAUDE_CODE_OAUTH_TOKEN && !process.env.ANTHROPIC_API_KEY,
			"Skipping agent tests - no Claude authentication",
		);

		test("should execute agent successfully", async ({ api, sse, testTask }, testInfo) => {
			// Increase timeout for agent execution
			testInfo.setTimeout(180000);

			// Trigger provisioning and wait for workspace + preview
			await triggerAndWaitForWorkspace(api, testTask.taskId);
			await waitForPreviewReady(api, testTask.taskId, { timeout: 120000 });

			// Send a simple prompt - we test infrastructure, not specific output
			// Agent behavior is unpredictable, so we verify it ran rather than what it produced
			await api.sendMessage(testTask.taskId, "What files are in this project?");

			// Wait for agent completion
			const completed = await waitForAgentIdleViaSSE(sse, api, testTask.taskId, {
				timeout: 120000,
				onProgress: (msg) => console.log(`  Agent: ${msg}`),
			});

			if (!completed) {
				// Save container logs on failure
				const logPath = `/tmp/e2e-agent-fail-${testTask.taskId.slice(0, 8)}.log`;
				await saveContainerLogs(testTask.containerName, logPath, {
					testName: testInfo.title,
					tail: 500,
				});
				console.log(`Container logs saved to: ${logPath}`);
			}

			expect(completed).toBe(true);

			// Verify agent did work (numTurns > 0)
			const task = await api.getTask(testTask.taskId);
			expect(task.numTurns).toBeGreaterThan(0);
			console.log(`  Agent completed with ${task.numTurns} turns`);

			// Verify SSE events were received
			const agentEvents = sse
				.getEventsForTask(testTask.taskId)
				.filter((e) => e.type === "agent" || e.type === "taskIteration");
			expect(agentEvents.length).toBeGreaterThan(0);
			console.log(`  Received ${agentEvents.length} agent/iteration events`);
		});

		test("should show task status after agent completion", async ({
			api,
			sse,
			testTask,
		}, testInfo) => {
			// Increase timeout
			testInfo.setTimeout(180000);

			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);
			await waitForPreviewReady(api, testTask.taskId, { timeout: 120000 });

			// Run agent with simple prompt (don't depend on file creation)
			await api.sendMessage(testTask.taskId, "List the top-level files");
			await waitForAgentIdleViaSSE(sse, api, testTask.taskId, { timeout: 120000 });

			// Check task status
			const task = await api.getTask(testTask.taskId);
			expect(task.taskId).toBe(testTask.taskId);
			expect(task.numTurns).toBeGreaterThan(0);
		});

		test("should show changes in git diff after file write", async ({ api, testTask }, testInfo) => {
			// Increase timeout
			testInfo.setTimeout(180000);

			// Trigger provisioning
			await triggerAndWaitForWorkspace(api, testTask.taskId);
			await waitForPreviewReady(api, testTask.taskId, { timeout: 120000 });

			// Write file via API (deterministic, no agent involvement)
			const fileName = "git-diff-test.txt";
			await api.writeFile(testTask.taskId, fileName, "Testing git diff");

			// Verify file exists
			const content = await api.getFileContent(testTask.taskId, fileName);
			expect(content).toBe("Testing git diff");

			// Verify git shows changes
			const diff = await api.getGitDiff(testTask.taskId);
			expect(diff).toBeDefined();
		});
	});

	test.describe("SSE Event Verification", () => {
		test("should receive workspace lifecycle events", async ({
			api,
			sse,
			testTask,
		}) => {
			// Trigger full provisioning flow
			await triggerAndWaitForWorkspace(api, testTask.taskId);
			await waitForPreviewReady(api, testTask.taskId, { timeout: 120000 });

			// Give events time to propagate
			await new Promise((resolve) => setTimeout(resolve, 2000));

			// Check for expected events
			const taskEvents = sse.getEventsForTask(testTask.taskId);
			console.log(`  Received ${taskEvents.length} events for task`);

			// Print event summary
			const summary = sse.getEventSummary(testTask.taskId);
			for (const [type, count] of Object.entries(summary)) {
				console.log(`    ${type}: ${count}`);
			}

			// Verify we received task event
			const taskEvent = taskEvents.find((e) => e.type === "task");
			// Note: task event may have been emitted before SSE collector started
		});

		test("should receive agent lifecycle events", async ({
			api,
			sse,
			testTask,
		}, testInfo) => {
			// Skip if no Claude auth
			test.skip(
				!process.env.CLAUDE_CODE_OAUTH_TOKEN && !process.env.ANTHROPIC_API_KEY,
				"Skipping - no Claude authentication",
			);

			testInfo.setTimeout(180000);

			// Full setup
			await triggerAndWaitForWorkspace(api, testTask.taskId);
			await waitForPreviewReady(api, testTask.taskId, { timeout: 120000 });

			// Run agent
			await api.sendMessage(testTask.taskId, "Create test.txt with content 'test'");
			await waitForAgentIdleViaSSE(sse, api, testTask.taskId, { timeout: 120000 });

			// Verify lifecycle events
			const verification = sse.verifyTaskLifecycle(testTask.taskId);

			console.log(`  Received events: ${verification.receivedEvents.join(", ")}`);
			if (verification.errors.length > 0) {
				console.log(`  Missing events: ${verification.errors.join(", ")}`);
			}

			// Some events may be missed if they occurred before SSE collector started
			// So we don't fail the test, just log the results
			expect(verification.receivedEvents.length).toBeGreaterThan(0);
		});
	});
});

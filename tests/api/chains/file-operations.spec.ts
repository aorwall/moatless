/**
 * Chain: File Operations (file-operations.md)
 *
 * Tests workspace file API: listing, reading, tree, search, and write.
 * Requires a sandbox in ready state but no agent execution.
 *
 * Chain:
 *   SANDBOX-01 (precondition) →
 *   FILE-01 → FILE-02 → FILE-03 → FILE-04 → FILE-05
 */

import { test, expect } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import {
	createTaskWithoutMessage,
	waitForSandboxReady,
	listFiles,
	readFile,
	getFileTree,
	searchFilesByName,
	writeFile,
} from "../steps";

test.describe("Chain: File Operations", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;

		// Create task without message (agent stays idle)
		const { taskId, task } = await createTaskWithoutMessage(api, repo.id);
		ctx.taskId = taskId;
		ctx.task = task;

		console.log(`[File Ops] Task created: ${taskId}`);
	});

	test.afterAll(async ({ api }) => {
		if (ctx.taskId) {
			await api.stopTask(ctx.taskId).catch(() => {});
			await api.deleteTask(ctx.taskId).catch(() => {});
		}
	});

	// ─── Precondition: Sandbox ready ────────────────────────────────────

	test("SANDBOX-01: Sandbox provisions for file operations", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await waitForSandboxReady(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status).not.toBeNull();
		ctx.sandboxStatus = status!;

		console.log(`[File Ops] Sandbox ready: ${status?.environmentStatus}`);
	});

	// ─── FILE-01: List workspace files ──────────────────────────────────

	test("FILE-01: List workspace files", async ({ api }, testInfo) => {
		testInfo.setTimeout(60000);

		// Poll until files appear (workspace setup takes a few seconds)
		let files: string[] = [];
		const deadline = Date.now() + 45000;

		while (Date.now() < deadline) {
			files = await listFiles(api, ctx.taskId!, ".", false);
			if (files.length > 0) {
				break;
			}
			await new Promise((r) => setTimeout(r, 2000));
		}

		expect(files.length).toBeGreaterThan(0);

		// Check for known template files
		const hasPackageJson = files.some((f) => f === "package.json" || f.endsWith("/package.json"));
		expect(hasPackageJson).toBe(true);

		console.log(`[File Ops] Listed ${files.length} files`);
	});

	// ─── FILE-02: Read workspace file ───────────────────────────────────

	test("FILE-02: Read workspace file", async ({ api }) => {
		const content = await readFile(api, ctx.taskId!, "package.json");

		expect(content).not.toBeNull();
		expect(content!.length).toBeGreaterThan(0);

		// package.json should be valid JSON
		const parsed = JSON.parse(content!);
		expect(parsed.name).toBeDefined();

		console.log(`[File Ops] Read package.json (${content!.length} chars)`);
	});

	// ─── FILE-03: File tree with git status overlay ─────────────────────

	test("FILE-03: File tree with git status overlay", async ({ api }) => {
		const tree = await getFileTree(api, ctx.taskId!);

		expect(tree).toBeDefined();
		expect(tree.tree).toBeDefined();
		expect(tree.tree.length).toBeGreaterThan(0);

		console.log(`[File Ops] File tree has ${tree.tree.length} root entries`);
	});

	// ─── FILE-04: Search files by name ──────────────────────────────────

	test("FILE-04: Search files by name", async ({ api }) => {
		const result = await searchFilesByName(api, ctx.taskId!, "package.json");

		expect(result).toBeDefined();
		expect(result.files.length).toBeGreaterThan(0);

		const found = result.files.some((f) => f.name === "package.json" || f.path.endsWith("package.json"));
		expect(found).toBe(true);

		console.log(`[File Ops] Search found ${result.files.length} matches`);
	});

	// ─── FILE-05: Write file to workspace ───────────────────────────────

	test("FILE-05: Write file to workspace", async ({ api }) => {
		const testContent = `// E2E test file written at ${new Date().toISOString()}\nexport const hello = "world";\n`;

		await writeFile(api, ctx.taskId!, "e2e-test-file.ts", testContent);

		// Verify by reading back
		const content = await readFile(api, ctx.taskId!, "e2e-test-file.ts");

		expect(content).not.toBeNull();
		expect(content).toContain("E2E test file");
		expect(content).toContain('export const hello = "world"');

		console.log("[File Ops] Write + read-back verified");
	});
});

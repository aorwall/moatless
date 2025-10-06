import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";

/**
 * Utility functions for setting up test repositories
 */

export interface TestRepository {
	id: string;
	name: string;
	tempDir: string;
}

export interface TestTask {
	taskId: string;
	repositoryId: string;
	agentType: string;
}

/**
 * Set up a temporary git repository for testing
 */
export async function setupTempGitRepo(tempDir: string): Promise<void> {
	// Initialize git repository
	await runCommand("git", ["init"], tempDir);
	await runCommand("git", ["config", "user.name", "E2E Test User"], tempDir);
	await runCommand(
		"git",
		["config", "user.email", "e2e-test@example.com"],
		tempDir,
	);

	// Create initial README
	const readmePath = path.join(tempDir, "README.md");
	await fs.writeFile(
		readmePath,
		"# E2E Test Repository\\n\\nThis is a test repository for comprehensive E2E testing.\\n",
	);

	// Add and commit initial file
	await runCommand("git", ["add", "README.md"], tempDir);
	await runCommand(
		"git",
		["commit", "-m", "Initial commit for E2E test"],
		tempDir,
	);
}

/**
 * Create a test repository via API
 */
export async function createTestRepository(
	baseURL: string,
	name: string,
	tempDir: string,
): Promise<TestRepository> {
	const response = await fetch(`${baseURL}/api/v1/repositories`, {
		method: "POST",
		headers: {
			"Content-Type": "application/json",
		},
		body: JSON.stringify({
			name,
			description: `E2E Test Repository - ${name}`,
			path: tempDir,
			useGitWorktrees: false,
			codeExecution: {
				type: "local",
				setupCommands: ['echo "E2E Repository setup complete"'],
			},
		}),
	});

	if (!response.ok) {
		throw new Error(
			`Failed to create repository: ${response.status} ${response.statusText}`,
		);
	}

	const repository = await response.json();
	return {
		id: repository.id,
		name: repository.name,
		tempDir,
	};
}

/**
 * Create a test task via API
 */
export async function createTestTask(
	baseURL: string,
	repositoryId: string,
	message: string,
	agentType = "claude-code",
	start = true,
): Promise<TestTask> {
	const response = await fetch(`${baseURL}/api/v1/tasks`, {
		method: "POST",
		headers: {
			"Content-Type": "application/json",
		},
		body: JSON.stringify({
			agentType,
			repositoryId,
			message,
			start,
		}),
	});

	if (!response.ok) {
		throw new Error(
			`Failed to create task: ${response.status} ${response.statusText}`,
		);
	}

	const result = await response.json();
	return {
		taskId: result.taskId,
		repositoryId,
		agentType,
	};
}

/**
 * Delete a test task via API
 */
export async function deleteTestTask(
	baseURL: string,
	taskId: string,
): Promise<void> {
	try {
		// First try to stop the task
		await fetch(`${baseURL}/api/v1/tasks/${taskId}/stop`, {
			method: "POST",
		});
		// Give it a moment to stop
		await new Promise((resolve) => setTimeout(resolve, 1000));
	} catch (error) {
		// Task might not be running
	}

	try {
		const response = await fetch(`${baseURL}/api/v1/tasks/${taskId}`, {
			method: "DELETE",
		});
		if (!response.ok && response.status !== 404) {
			console.warn(`Failed to delete task ${taskId}: ${response.status}`);
		}
	} catch (error) {
		console.warn(`Error deleting task ${taskId}:`, error);
	}
}

/**
 * Delete a test repository via API
 */
export async function deleteTestRepository(
	baseURL: string,
	repositoryId: string,
): Promise<void> {
	try {
		const response = await fetch(
			`${baseURL}/api/v1/repositories/${repositoryId}`,
			{
				method: "DELETE",
			},
		);
		if (!response.ok && response.status !== 404) {
			console.warn(
				`Failed to delete repository ${repositoryId}: ${response.status}`,
			);
		}
	} catch (error) {
		console.warn(`Error deleting repository ${repositoryId}:`, error);
	}
}

/**
 * Wait for a task to complete by polling for result events
 */
export async function waitForTaskCompletion(
	baseURL: string,
	taskId: string,
	maxWaitMs = 60000,
): Promise<{ success: boolean; result?: any; error?: string }> {
	const startTime = Date.now();
	const pollInterval = 1000; // 1 second

	while (Date.now() - startTime < maxWaitMs) {
		try {
			const response = await fetch(`${baseURL}/api/v1/tasks/${taskId}/events`);
			if (response.ok) {
				const { events } = await response.json();

				// Look for result events
				for (const event of events) {
					if (event.type === "result") {
						if (event.isError) {
							return { success: false, error: event.payload };
						}
						return { success: true, result: event.payload };
					}
				}
			}
		} catch (error) {
			console.warn("Error polling for task completion:", error);
		}

		await new Promise((resolve) => setTimeout(resolve, pollInterval));
	}

	return { success: false, error: "Timeout waiting for task completion" };
}

/**
 * Helper function to run shell commands
 */
async function runCommand(
	command: string,
	args: string[],
	cwd: string,
): Promise<void> {
	return new Promise((resolve, reject) => {
		const child = spawn(command, args, {
			cwd,
			stdio: "pipe",
			env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
		});

		let stderr = "";
		child.stderr?.on("data", (data: Buffer) => {
			stderr += data.toString();
		});

		child.on("close", (code: number) => {
			if (code !== 0) {
				reject(
					new Error(
						`Command ${command} ${args.join(" ")} failed with code ${code}: ${stderr}`,
					),
				);
			} else {
				resolve();
			}
		});

		child.on("error", reject);
	});
}

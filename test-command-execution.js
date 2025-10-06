#!/usr/bin/env node

/**
 * Test script for command execution endpoints
 * Usage: node test-command-execution.js
 */

const API_BASE_URL = "http://localhost:8000/api/v1";

async function testRepositoryCommandExecution() {
	console.log("🧪 Testing repository command execution...\n");

	// First, get the list of repositories
	const reposResponse = await fetch(`${API_BASE_URL}/repositories`);
	const repos = await reposResponse.json();

	if (!repos.repositories || repos.repositories.length === 0) {
		console.log("❌ No repositories found. Please create a repository first.");
		return;
	}

	const repo = repos.repositories[0];
	console.log(`📦 Using repository: ${repo.name} (${repo.id})\n`);

	// Test command execution with SSE
	const commands = {
		commands: ['echo "Testing command execution"', "pwd", "ls -la"],
		purpose: "test",
		continueOnError: true,
		timeout: 30000,
	};

	console.log("📤 Sending command execution request...");
	console.log("Commands:", commands.commands);
	console.log();

	const response = await fetch(
		`${API_BASE_URL}/repositories/${repo.id}/execute-commands`,
		{
			method: "POST",
			headers: {
				"Content-Type": "application/json",
			},
			body: JSON.stringify(commands),
		},
	);

	if (!response.ok) {
		console.error(
			"❌ Failed to execute commands:",
			response.status,
			response.statusText,
		);
		return;
	}

	console.log("📥 Receiving SSE stream...\n");

	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let buffer = "";

	while (true) {
		const { done, value } = await reader.read();
		if (done) break;

		buffer += decoder.decode(value, { stream: true });

		// Process complete SSE messages
		const lines = buffer.split("\n");
		buffer = lines.pop() || ""; // Keep incomplete line in buffer

		for (const line of lines) {
			if (line.startsWith("event: ")) {
				const eventType = line.substring(7);
				console.log(`📌 Event: ${eventType}`);
			} else if (line.startsWith("data: ")) {
				try {
					const data = JSON.parse(line.substring(6));
					console.log("📊 Data:", JSON.stringify(data, null, 2));
				} catch (e) {
					console.log("📊 Data:", line.substring(6));
				}
			}
		}
	}

	console.log("\n✅ Command execution test completed!");
}

async function main() {
	console.log("=================================");
	console.log("Command Execution Endpoint Tester");
	console.log("=================================\n");

	try {
		// Check if backend is running by trying to get repositories
		try {
			const testResponse = await fetch(`${API_BASE_URL}/repositories`);
			if (!testResponse.ok) {
				throw new Error("Backend is not responding");
			}
			console.log("✅ Backend is running\n");
		} catch (e) {
			throw new Error("Backend is not responding");
		}

		await testRepositoryCommandExecution();
	} catch (error) {
		console.error("❌ Error:", error.message);
		console.log("\nMake sure the backend is running: bun run dev");
	}
}

// Run the test
main().catch(console.error);

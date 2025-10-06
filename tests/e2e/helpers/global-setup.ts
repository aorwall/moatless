import { type ChildProcess, spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";
import type { FullConfig } from "@playwright/test";
import dotenv from "dotenv";

// Load environment variables from .env file
dotenv.config();

// Store server process info globally for teardown
declare global {
	var __E2E_SERVER_PID__: number | undefined;
	var __E2E_SERVER_PORT__: number | undefined;
}

let serverProcess: ChildProcess | null = null;
let tempDir: string;

/**
 * Find an available port starting from the given port number
 */
async function findAvailablePort(startPort = 3002): Promise<number> {
	for (let port = startPort; port < startPort + 100; port++) {
		if (await isPortAvailable(port)) {
			return port;
		}
	}
	throw new Error(
		`No available port found in range ${startPort}-${startPort + 99}`,
	);
}

/**
 * Check if a port is available by trying to create a server on it
 */
function isPortAvailable(port: number): Promise<boolean> {
	return new Promise((resolve) => {
		const server = createServer();

		server.listen(port, () => {
			server.close(() => {
				resolve(true);
			});
		});

		server.on("error", () => {
			resolve(false);
		});
	});
}

async function globalSetup(config: FullConfig) {
	// Check if Claude OAuth token is set
	if (!process.env.CLAUDE_CODE_OAUTH_TOKEN) {
		console.warn(
			"⚠️  WARNING: CLAUDE_CODE_OAUTH_TOKEN is not set in environment variables.",
		);
		console.warn(
			"   E2E tests that use Claude Code agent will fail without this token.",
		);
		console.warn(
			"   Set it in your .env file or environment: CLAUDE_CODE_OAUTH_TOKEN=your-token",
		);
		console.warn("");
		throw new Error("E2E tests require a Claude Code OAuth token");
	}

	// Create a persistent temporary directory for test data (not removed after tests)
	const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
	tempDir = path.join(os.tmpdir(), `moatless-e2e-data-${timestamp}`);
	await fs.mkdir(tempDir, { recursive: true });
	console.log("Created persistent temp directory:", tempDir);
	console.log(
		"Note: This directory will not be automatically removed after tests complete",
	);

	// Set variables for test
	process.env.SKIP_CLAUDE_DISK_EVENTS = "true";
	process.env.USE_CONSOLE_LOGGING = "true";
	process.env.NODE_ENV = "test";
	process.env.E2E_TEMP_DIR = tempDir;

	// Find an available port first (before build)
	const availablePort = await findAvailablePort(
		process.env.E2E_PORT ? Number.parseInt(process.env.E2E_PORT, 10) : 3002,
	);

	// Set API URL for the build process
	process.env.VITE_API_URL = `http://127.0.0.1:${availablePort}`;

	// Build the application first
	console.log(
		`Building application for E2E tests with API URL: ${process.env.VITE_API_URL}...`,
	);
	const buildProcess = spawn("bun", ["run", "build"], {
		stdio: "inherit",
		env: process.env,
	});

	await new Promise<void>((resolve, reject) => {
		buildProcess.on("close", (code) => {
			if (code === 0) {
				resolve();
			} else {
				reject(new Error(`Build failed with code ${code}`));
			}
		});
		buildProcess.on("error", reject);
	});

	// Store the port globally for tests to use
	globalThis.__E2E_SERVER_PORT__ = availablePort;

	// Set the base URL for Playwright config
	process.env.E2E_BASE_URL = `http://127.0.0.1:${availablePort}`;

	// Start the server
	console.log(`Starting server for E2E tests on port ${availablePort}...`);
	serverProcess = spawn("node", ["apps/backend/dist/index.js"], {
		env: {
			...process.env,
			PORT: availablePort.toString(),
			NODE_ENV: "production", // Use production mode to enable static file serving
			LOG_LEVEL: "debug", // Enable debug logging for E2E tests
			SKIP_CLAUDE_DISK_EVENTS: "true",
			USE_CONSOLE_LOGGING: "true",
			DATA_DIR: tempDir, // Use the temporary directory for data storage
		},
		stdio: "pipe",
	});

	// Wait for server to be ready
	await new Promise<void>((resolve, reject) => {
		let serverReady = false;
		const timeout = setTimeout(() => {
			if (!serverReady) {
				reject(new Error("Server startup timeout"));
			}
		}, 30000); // 30 second timeout

		if (serverProcess?.stdout) {
			serverProcess.stdout.on("data", (data) => {
				const output = data.toString();
				console.log("Server:", output.trim());

				// Look for server ready indicators
				if (
					output.includes("Server listening on") ||
					output.includes("listening on port")
				) {
					serverReady = true;
					clearTimeout(timeout);
					resolve();
				}
			});
		}

		if (serverProcess?.stderr) {
			serverProcess.stderr.on("data", (data) => {
				console.error("Server error:", data.toString().trim());
			});
		}

		serverProcess?.on("error", (error) => {
			clearTimeout(timeout);
			reject(error);
		});

		serverProcess?.on("exit", (code) => {
			clearTimeout(timeout);
			if (!serverReady) {
				reject(
					new Error(`Server exited with code ${code} before becoming ready`),
				);
			}
		});
	});

	// Store the server PID for cleanup
	if (serverProcess?.pid) {
		globalThis.__E2E_SERVER_PID__ = serverProcess.pid;
		console.log(
			`E2E setup complete. Server running on port ${availablePort} with PID: ${serverProcess.pid}`,
		);
	} else {
		console.log(`E2E setup complete. Server running on port ${availablePort}`);
	}

	// Give the server a moment to fully initialize
	await new Promise((resolve) => setTimeout(resolve, 2000));
}

export default globalSetup;

#!/usr/bin/env node

import { spawn } from "child_process";
import { createServer } from "net";

// Function to check if a port is available
async function isPortAvailable(port) {
	return new Promise((resolve) => {
		const server = createServer();

		server.once("error", (err) => {
			if (err.code === "EADDRINUSE") {
				resolve(false);
			} else {
				resolve(false);
			}
		});

		server.once("listening", () => {
			server.close();
			resolve(true);
		});

		server.listen(port, "0.0.0.0");
	});
}

// Function to find an available port
async function findAvailablePort(startPort = 8000) {
	let port = startPort;
	while (port < startPort + 100) {
		// Try up to 100 ports
		if (await isPortAvailable(port)) {
			return port;
		}
		console.log(`Port ${port} is in use, trying ${port + 1}...`);
		port++;
	}
	throw new Error(
		`No available ports found between ${startPort} and ${startPort + 100}`,
	);
}

async function waitForBackend(port, maxAttempts = 30) {
	console.log(`⏳ Waiting for backend to start on port ${port}...`);

	for (let i = 0; i < maxAttempts; i++) {
		try {
			const response = await fetch(`http://localhost:${port}/health`).catch(
				() => null,
			);
			if (response && response.ok) {
				console.log(`✅ Backend is ready on port ${port}`);
				return true;
			}
		} catch (e) {
			// Server not ready yet
		}

		// Check if port is now in use (backend started)
		if (!(await isPortAvailable(port))) {
			// Port is in use, give it a moment to fully initialize
			await new Promise((resolve) => setTimeout(resolve, 1000));
			return true;
		}

		await new Promise((resolve) => setTimeout(resolve, 1000));
	}

	return false;
}

async function main() {
	try {
		// Find an available port
		const port = await findAvailablePort(8000);
		console.log(`✅ Found available port: ${port}`);
		console.log(`🚀 Starting backend on port ${port}`);

		// Start the backend server with the found port
		const backendProcess = spawn(
			"bun",
			["--filter", "@moatless/backend", "dev"],
			{
				stdio: "inherit",
				env: {
					...process.env,
					PORT: port.toString(),
					USE_CONSOLE_LOGGING: "true",
				},
				shell: true,
			},
		);

		// Wait for backend to be ready
		const backendReady = await waitForBackend(port);

		if (!backendReady) {
			console.error("❌ Backend failed to start within 30 seconds");
			backendProcess.kill();
			process.exit(1);
		}

		// Start the frontend with the backend port as an environment variable
		console.log(`🚀 Starting frontend with API URL: http://localhost:${port}`);
		const frontendEnv = {
			...process.env,
			VITE_API_URL: `http://localhost:${port}`,
		};

		// If PORT is provided in the environment, pass it to the frontend
		if (process.env.PORT) {
			frontendEnv.PORT = process.env.PORT;
			console.log(`   Frontend will use port: ${process.env.PORT}`);
		}

		const frontendProcess = spawn(
			"bun",
			["--filter", "@moatless/frontend", "dev"],
			{
				stdio: "inherit",
				env: frontendEnv,
				shell: true,
			},
		);

		// Handle process termination
		const cleanup = () => {
			console.log("\n🛑 Shutting down...");
			backendProcess.kill();
			frontendProcess.kill();
			process.exit();
		};

		process.on("SIGINT", cleanup);
		process.on("SIGTERM", cleanup);

		// Handle child process errors
		backendProcess.on("error", (err) => {
			console.error("Backend process error:", err);
			cleanup();
		});

		frontendProcess.on("error", (err) => {
			console.error("Frontend process error:", err);
			cleanup();
		});

		// Handle backend exit
		backendProcess.on("exit", (code) => {
			if (code !== 0 && code !== null) {
				console.error(`Backend exited with code ${code}`);
				cleanup();
			}
		});
	} catch (error) {
		console.error("Error starting development servers:", error);
		process.exit(1);
	}
}

main();

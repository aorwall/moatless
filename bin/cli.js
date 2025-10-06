#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const rootDir = join(__dirname, "..");

// Parse command line arguments
const args = process.argv.slice(2);
const command = args[0] || "start";

// Environment setup
process.env.NODE_ENV = process.env.NODE_ENV || "production";

switch (command) {
	case "start":
		startServer();
		break;
	case "dev":
		startDevelopment();
		break;
	default:
		console.log(`Unknown command: ${command}`);
		console.log("Usage: npx moatless-vibe [start|dev]");
		process.exit(1);
}

function startServer() {
	// Check if running from installed package or development
	const isInstalledPackage = !existsSync(join(rootDir, "apps"));
	const serverPath = isInstalledPackage
		? join(rootDir, "dist", "server.js")
		: join(rootDir, "apps", "backend", "dist", "index.js");

	// Set static client path for production
	if (isInstalledPackage) {
		process.env.STATIC_CLIENT_PATH = join(rootDir, "dist", "client");
	}

	const child = spawn("node", [serverPath], {
		cwd: rootDir,
		stdio: "inherit",
		env: { ...process.env },
	});

	child.on("error", (err) => {
		console.error("Failed to start server:", err);
		process.exit(1);
	});

	child.on("exit", (code) => {
		process.exit(code || 0);
	});
}

function startDevelopment() {
	const child = spawn("bun", ["dev"], {
		cwd: rootDir,
		stdio: "inherit",
		shell: true,
	});

	child.on("error", (err) => {
		console.error("Failed to start development server:", err);
		process.exit(1);
	});

	child.on("exit", (code) => {
		process.exit(code || 0);
	});
}

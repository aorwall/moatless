#!/usr/bin/env node

import { execSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";

const rootDir = process.cwd();
const distDir = join(rootDir, "dist");

// Clean dist directory
if (existsSync(distDir)) {
	rmSync(distDir, { recursive: true, force: true });
}
mkdirSync(distDir, { recursive: true });

// Bundle backend with esbuild
console.log("🔨 Bundling backend...");
execSync("node scripts/build-backend.js", { stdio: "inherit" });

// Copy frontend build
const frontendDist = join(rootDir, "apps/frontend/dist");
const clientDist = join(distDir, "client");
cpSync(frontendDist, clientDist, { recursive: true });

// Copy drizzle migrations to root (needed for production deployments)
const drizzleSource = join(rootDir, "apps/backend/drizzle");
const drizzleDest = join(rootDir, "drizzle");
if (existsSync(drizzleSource)) {
	console.log("📋 Copying drizzle migrations...");
	cpSync(drizzleSource, drizzleDest, { recursive: true });
	console.log("✅ Drizzle migrations copied successfully");
} else {
	console.warn("⚠️  Drizzle migrations folder not found at:", drizzleSource);
}

console.log("✅ Package prepared for publishing");
console.log("📦 Distribution files created in:", distDir);

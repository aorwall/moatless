#!/usr/bin/env node

import { cpSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import esbuild from "esbuild";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const rootDir = join(__dirname, "..");

async function buildBackend() {
	console.log("🔨 Bundling backend with esbuild...");

	try {
		await esbuild.build({
			entryPoints: [join(rootDir, "apps/backend/src/index.ts")],
			bundle: true,
			platform: "node",
			target: "node22",
			format: "esm",
			outfile: join(rootDir, "dist/server.js"),
			external: [
				// Keep native modules external
				"dockerode",
				"@anthropic-ai/claude-code",
				// Keep pino-pretty external (has native dependencies)
				"pino-pretty",
				// Keep playwright external (has native dependencies)
				"playwright-core",
				"playwright",
				"chromium-bidi",
				"fsevents",
				// Keep Bun built-ins external
				"bun:sqlite",
			],
			loader: {
				".ts": "ts",
				".tsx": "tsx",
			},
			sourcemap: true,
			minify: process.env.NODE_ENV === "production",
			define: {
				"process.env.NODE_ENV": JSON.stringify(
					process.env.NODE_ENV || "production",
				),
			},
			inject: ["./scripts/esm-shims.js"],
		});

		console.log("✅ Backend bundled successfully");

		// Copy preview-inspector.js to dist/features/preview/inspector/assets/
		console.log("📋 Copying preview-inspector.js...");
		const assetsDir = join(rootDir, "dist/features/preview/inspector/assets");
		mkdirSync(assetsDir, { recursive: true });
		cpSync(
			join(
				rootDir,
				"apps/backend/src/features/preview/inspector/assets/preview-inspector.js",
			),
			join(assetsDir, "preview-inspector.js"),
		);
		console.log("✅ preview-inspector.js copied successfully");
	} catch (error) {
		console.error("❌ Backend bundling failed:", error);
		process.exit(1);
	}
}

buildBackend();

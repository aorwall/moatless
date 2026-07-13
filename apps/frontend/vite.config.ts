import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig, loadEnv } from "vite";
import tsconfigPaths from "vite-tsconfig-paths";

/**
 * Vite plugin that exchanges an API key for a session cookie on startup.
 * This lets local dev servers authenticate against a remote backend without
 * needing OAuth callbacks (which redirect to the remote URL, not localhost).
 */
function apiKeyAuthPlugin(target: string, apiKey: string): Plugin {
	let sessionCookie: string | null = null;

	return {
		name: "api-key-auth",
		configureServer(server) {
			// Exchange API key for session when the dev server starts
			server.httpServer?.once("listening", async () => {
				try {
					const res = await fetch(
						`${target}/api/v1/auth/api-key/session`,
						{
							method: "POST",
							headers: { Authorization: `Bearer ${apiKey}` },
						},
					);
					if (!res.ok) {
						console.warn(
							`API key auth failed: ${res.status} ${res.statusText}`,
						);
						return;
					}
					const setCookie = res.headers.get("set-cookie");
					if (setCookie) {
						sessionCookie = setCookie.split(";")[0];
						console.log("API key auth: session cookie acquired");
					}
				} catch (e) {
					console.warn("API key auth exchange failed:", e);
				}
			});

			// Inject session cookie on all proxied /api requests
			server.middlewares.use((req, _res, next) => {
				if (sessionCookie && req.url?.startsWith("/api")) {
					req.headers.cookie = sessionCookie;
				}
				next();
			});
		},
	};
}

export default defineConfig(({ mode }) => {
	const env = loadEnv(mode, process.cwd(), "VITE_");
	const remoteApi = env.VITE_REMOTE_API;
	const apiKey = env.VITE_API_KEY;
	const target = remoteApi || "http://localhost:8080";
	const isRemote = remoteApi && !remoteApi.includes("localhost");

	return {
		plugins: [
			react({
				babel: {
					// React Compiler (C2 experiment, docs/frontend-improvement-plan-2026-07.md).
					// Auto-memoizes components/hooks; un-compilable ones are skipped safely.
					plugins: [["babel-plugin-react-compiler", {}]],
				},
			}),
			tsconfigPaths(),
			tailwindcss(),
			tanstackRouter({
				routesDirectory: "./src/routes",
				generatedRouteTree: "./src/routeTree.gen.ts",
				routeFileIgnorePattern:
					"\\.test\\.(ts|tsx|js|jsx)$|\\.spec\\.(ts|tsx|js|jsx)$",
			}),
			...(isRemote && apiKey ? [apiKeyAuthPlugin(target, apiKey)] : []),
		],
		server: {
			port: process.env.PORT ? Number(process.env.PORT) : 3000,
			allowedHosts: true,
			proxy: {
				"/api": {
					target,
					changeOrigin: true,
					secure: false,
					...(isRemote && { cookieDomainRewrite: "" }),
				},
			},
		},
		build: {
			outDir: "dist",
			// Do not publish source maps with the production bundle. Public maps
			// disclose original source structure, identifiers, and internal API
			// usage (security assessment 2026-07-13). Dev/preview builds still get
			// inline maps via Vite's defaults; only the deployed `build` output is
			// stripped. Use "hidden" + an authenticated error-monitoring upload
			// pipeline if maps are needed for production error triage.
			sourcemap: mode === "production" ? false : true,
			rollupOptions: {
				output: {
					manualChunks: {
						'vendor-react': ['react', 'react-dom'],
						'vendor-tanstack': ['@tanstack/react-query', '@tanstack/react-router'],
						'vendor-radix': ['@radix-ui/react-dialog', '@radix-ui/react-popover', '@radix-ui/react-select', '@radix-ui/react-scroll-area', '@radix-ui/react-tabs', '@radix-ui/react-tooltip'],
						'vendor-markdown': ['react-markdown', 'react-syntax-highlighter', 'remark-gfm'],
						'vendor-icons': ['lucide-react'],
					},
				},
			},
		},
		worker: {
			format: "es",
		},
		appType: "spa",
	};
});

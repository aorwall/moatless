import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import tsconfigPaths from "vite-tsconfig-paths";

export default defineConfig({
	plugins: [
		react(),
		tsconfigPaths(),
		tailwindcss(),
		tanstackRouter({
			routesDirectory: "./src/routes",
			generatedRouteTree: "./src/routeTree.gen.ts",
			routeFileIgnorePattern:
				"\\.test\\.(ts|tsx|js|jsx)$|\\.spec\\.(ts|tsx|js|jsx)$",
		}),
	],
	server: {
		port: process.env.PORT ? Number(process.env.PORT) : 3000,
		proxy: {
			"/api": {
				target: "http://localhost:8000",
				changeOrigin: true,
			},
		},
	},
	build: {
		outDir: "dist",
		sourcemap: true,
		rollupOptions: {
			output: {
				manualChunks: {
					'vendor-react': ['react', 'react-dom'],
					'vendor-tanstack': ['@tanstack/react-query', '@tanstack/react-router'],
					'vendor-radix': ['@radix-ui/react-dialog', '@radix-ui/react-popover', '@radix-ui/react-select', '@radix-ui/react-scroll-area', '@radix-ui/react-tabs', '@radix-ui/react-tooltip'],
					'vendor-markdown': ['react-markdown', 'react-syntax-highlighter', 'remark-gfm'],
				},
			},
		},
	},
	appType: "spa",
});

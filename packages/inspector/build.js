import { build } from 'esbuild'
import { mkdir } from 'node:fs/promises'

// Ensure dist directory exists
await mkdir('dist', { recursive: true })

// Bundle the client-side inspector into a single IIFE file
// This is the script that gets injected into preview iframes
await build({
	entryPoints: ['src/client/index.ts'],
	outfile: 'dist/client.js',
	bundle: true,
	format: 'iife',
	platform: 'browser',
	target: 'es2020',
	minify: true,
	sourcemap: false,
})

console.log('Build complete: dist/client.js')

import { build } from 'esbuild'
import { copyFile, mkdir } from 'node:fs/promises'

// Ensure dist directories exist
await mkdir('dist', { recursive: true })
await mkdir('dist/tagger', { recursive: true })

// The T3 annotation client arrives prebuilt from t3code (see
// scripts/sync-t3-annotation.mjs) — there is nothing to compile, only to ship.
await copyFile('vendor/t3-annotation-client.js', 'dist/t3-annotation-client.js')

console.log('Copied: dist/t3-annotation-client.js')

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

// Bundle tagger core (ESM)
await build({
	entryPoints: ['src/tagger/index.ts'],
	outfile: 'dist/tagger/index.js',
	bundle: true,
	format: 'esm',
	platform: 'node',
	target: 'node18',
	external: ['@babel/parser', 'estree-walker', 'magic-string', 'node:path'],
	sourcemap: true,
})

console.log('Build complete: dist/tagger/index.js')

// Bundle webpack loader (ESM)
await build({
	entryPoints: ['src/webpack.ts'],
	outfile: 'dist/webpack.js',
	bundle: true,
	format: 'esm',
	platform: 'node',
	target: 'node18',
	external: ['@babel/parser', 'estree-walker', 'magic-string'],
	sourcemap: true,
})

console.log('Build complete: dist/webpack.js')

// Bundle webpack loader (CJS) - for Next.js compatibility
// Note: We bundle estree-walker and magic-string since they're ESM-only packages
// and would fail when required from CJS context
await build({
	entryPoints: ['src/webpack.ts'],
	outfile: 'dist/webpack.cjs',
	bundle: true,
	format: 'cjs',
	platform: 'node',
	target: 'node18',
	external: ['@babel/parser'],
	sourcemap: true,
})

console.log('Build complete: dist/webpack.cjs')

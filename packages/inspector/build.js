import { build } from 'esbuild'
import { mkdir } from 'node:fs/promises'

// Ensure dist directories exist
await mkdir('dist', { recursive: true })
await mkdir('dist/tagger', { recursive: true })

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
await build({
	entryPoints: ['src/webpack.ts'],
	outfile: 'dist/webpack.cjs',
	bundle: true,
	format: 'cjs',
	platform: 'node',
	target: 'node18',
	external: ['@babel/parser', 'estree-walker', 'magic-string'],
	sourcemap: true,
})

console.log('Build complete: dist/webpack.cjs')

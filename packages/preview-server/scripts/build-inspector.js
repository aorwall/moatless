#!/usr/bin/env node

import { buildSync } from 'esbuild'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)
const packageRoot = join(__dirname, '..')

console.log('🔨 Building inspector.ts...')

try {
  buildSync({
    entryPoints: [join(packageRoot, 'src/scripts/inspector.ts')],
    bundle: true,
    platform: 'browser',
    target: 'es2020',
    format: 'iife',
    outfile: join(packageRoot, 'src/scripts/preview-inspector.js'),
    minify: true,
    sourcemap: false,
    logLevel: 'warning',
  })

  console.log('✅ preview-inspector.js built successfully')
} catch (error) {
  console.error('❌ Build failed:', error)
  process.exit(1)
}

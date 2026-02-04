/**
 * Build script for file-explorer.
 *
 * Phase 1: Bundle the frontend SPA (HTML + TSX + CSS/Tailwind) into dist/public/
 * Phase 2: Bundle the server into dist/server.js targeting Bun
 */
import { rmSync } from 'node:fs'
import tailwindPlugin from 'bun-plugin-tailwind'

const ROOT = import.meta.dir

// Clean previous build
rmSync(`${ROOT}/dist`, { recursive: true, force: true })

console.log('Phase 1: Building frontend SPA...')
const frontendResult = await Bun.build({
  entrypoints: [`${ROOT}/public/index.html`],
  outdir: `${ROOT}/dist/public`,
  minify: true,
  plugins: [tailwindPlugin],
})

if (!frontendResult.success) {
  console.error('Frontend build failed:')
  for (const log of frontendResult.logs) {
    console.error(log)
  }
  process.exit(1)
}
console.log(`  Frontend: ${frontendResult.outputs.length} files written to dist/public/`)

console.log('Phase 2: Building server...')
const serverResult = await Bun.build({
  entrypoints: [`${ROOT}/server.ts`],
  outdir: `${ROOT}/dist`,
  target: 'bun',
  minify: true,
  external: ['./public/index.html'],
  define: {
    'process.env.NODE_ENV': '"production"',
  },
})

if (!serverResult.success) {
  console.error('Server build failed:')
  for (const log of serverResult.logs) {
    console.error(log)
  }
  process.exit(1)
}
console.log(`  Server: ${serverResult.outputs.length} files written to dist/`)
console.log('Build complete!')

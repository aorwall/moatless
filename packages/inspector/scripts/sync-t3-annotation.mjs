#!/usr/bin/env node
/**
 * Refresh `vendor/t3-annotation-client.js` from a t3code checkout.
 *
 * The T3 annotation runtime is developed in soaplabs/t3code, not here — this
 * package only ships it, because the inspector plugin is what previewed
 * projects already install. Vendoring the built bundle keeps that one artifact
 * in the published tarball without forking 1300 lines of picker source or
 * adding a cross-repo package dependency.
 *
 *   node scripts/sync-t3-annotation.mjs [path-to-t3code]
 *
 * The path may also come from T3CODE_DIR; it defaults to a sibling checkout.
 */
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const t3codeDir = path.resolve(
  process.argv[2] ?? process.env.T3CODE_DIR ?? path.join(packageRoot, '../../../t3code'),
)
const sourcePackage = path.join(t3codeDir, 'packages/preview-annotation')

if (!existsSync(sourcePackage)) {
  console.error(`No t3code checkout at ${t3codeDir} (looked for packages/preview-annotation).`)
  console.error('Pass the checkout path as an argument or set T3CODE_DIR.')
  process.exit(1)
}

console.log(`Building @t3tools/preview-annotation in ${t3codeDir}...`)
execFileSync('pnpm', ['--filter', '@t3tools/preview-annotation', 'build'], {
  cwd: t3codeDir,
  stdio: 'inherit',
})

const built = path.join(sourcePackage, 'dist/preview-annotation.js')
const bundle = readFileSync(built, 'utf-8')

// Whatever revision produced these bytes is the only thing a reader of a
// minified bundle can act on, so it goes in the file.
const revision = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: t3codeDir, encoding: 'utf-8' })
const header = [
  '/*',
  ' * @t3tools/preview-annotation — the T3 Code element picker, built for the browser.',
  ' *',
  ' * Vendored, not hand-edited. Source: soaplabs/t3code packages/preview-annotation',
  ` * at ${revision.trim()}.`,
  ' * Regenerate with: node scripts/sync-t3-annotation.mjs [path-to-t3code]',
  ' */',
  '',
].join('\n')

const destination = path.join(packageRoot, 'vendor/t3-annotation-client.js')
mkdirSync(path.dirname(destination), { recursive: true })
writeFileSync(destination, header + bundle)

console.log(`Wrote ${destination} (${(bundle.length / 1024).toFixed(1)} kB)`)

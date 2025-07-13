#!/usr/bin/env node

import { cpSync, mkdirSync, rmSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { execSync } from 'node:child_process'

const rootDir = process.cwd()
const distDir = join(rootDir, 'dist')

// Clean dist directory
if (existsSync(distDir)) {
  rmSync(distDir, { recursive: true, force: true })
}
mkdirSync(distDir, { recursive: true })

// Bundle backend with esbuild
console.log('🔨 Bundling backend...')
execSync('node scripts/build-backend.js', { stdio: 'inherit' })

// Copy frontend build
const frontendDist = join(rootDir, 'apps/frontend/dist')
const clientDist = join(distDir, 'client')
cpSync(frontendDist, clientDist, { recursive: true })

console.log('✅ Package prepared for publishing')
console.log('📦 Distribution files created in:', distDir)
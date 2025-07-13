#!/usr/bin/env node

import { cpSync, mkdirSync, rmSync, existsSync } from 'node:fs'
import { join } from 'node:path'

const rootDir = process.cwd()
const distDir = join(rootDir, 'dist')

// Clean dist directory
if (existsSync(distDir)) {
  rmSync(distDir, { recursive: true, force: true })
}
mkdirSync(distDir, { recursive: true })

// Copy backend build
const backendDist = join(rootDir, 'apps/backend/dist')
const serverDist = join(distDir, 'server')
cpSync(backendDist, serverDist, { recursive: true })

// Copy frontend build
const frontendDist = join(rootDir, 'apps/frontend/dist')
const clientDist = join(distDir, 'client')
cpSync(frontendDist, clientDist, { recursive: true })

// Copy types package (needed for runtime)
const typesDist = join(rootDir, 'packages/types')
const typesTarget = join(distDir, 'packages/types')
mkdirSync(join(distDir, 'packages'), { recursive: true })
cpSync(typesDist, typesTarget, { 
  recursive: true,
  filter: (src) => !src.includes('node_modules') && !src.includes('__tests__')
})

console.log('✅ Package prepared for publishing')
console.log('📦 Distribution files created in:', distDir)
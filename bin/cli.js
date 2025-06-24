#\!/usr/bin/env node

import { spawn } from 'child_process'
import { dirname, join } from 'path'
import { fileURLToPath } from 'url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)
const rootDir = join(__dirname, '..')

// Parse command line arguments
const args = process.argv.slice(2)
const command = args[0] || 'start'

// Environment setup
process.env.NODE_ENV = process.env.NODE_ENV || 'production'

switch (command) {
  case 'start':
    startServer()
    break
  case 'dev':
    startDevelopment()
    break
  default:
    console.log(`Unknown command: ${command}`)
    console.log('Usage: npx moatless [start|dev]')
    process.exit(1)
}

function startServer() {
  const serverPath = join(rootDir, 'dist', 'server', 'index.js')
  const child = spawn('node', [serverPath], {
    cwd: rootDir,
    stdio: 'inherit',
    env: { ...process.env }
  })

  child.on('error', (err) => {
    console.error('Failed to start server:', err)
    process.exit(1)
  })

  child.on('exit', (code) => {
    process.exit(code || 0)
  })
}

function startDevelopment() {
  const child = spawn('pnpm', ['dev'], {
    cwd: rootDir,
    stdio: 'inherit',
    shell: true
  })

  child.on('error', (err) => {
    console.error('Failed to start development server:', err)
    process.exit(1)
  })

  child.on('exit', (code) => {
    process.exit(code || 0)
  })
}
EOF < /dev/null
import type { ChildProcess } from 'node:child_process'
import { execSync, spawn } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

const INSPECTOR_PKG_ROOT = path.resolve(__dirname, '../..')

async function waitForServer(url: string, timeoutMs: number): Promise<void> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    try {
      const response = await fetch(url)
      if (response.ok) return
    } catch {
      // Server not ready yet
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`Server at ${url} did not become ready within ${timeoutMs}ms`)
}

describe('Inspector Vite Plugin Integration', () => {
  let tmpDir: string
  let devProcess: ChildProcess
  const port = 5199 + Math.floor(Math.random() * 100)

  beforeAll(async () => {
    // 1. Clone moatable to temp dir
    tmpDir = mkdtempSync(path.join(os.tmpdir(), 'inspector-test-'))
    execSync(`git clone --depth 1 https://github.com/aorwall/moatable.git ${tmpDir}`, {
      stdio: 'pipe',
      timeout: 30_000,
    })

    // 2. Add @moatless/inspector dependency via file: protocol
    const pkgJsonPath = path.join(tmpDir, 'package.json')
    const pkgJson = JSON.parse(readFileSync(pkgJsonPath, 'utf-8'))
    pkgJson.dependencies['@moatless/inspector'] = `file:${INSPECTOR_PKG_ROOT}`
    writeFileSync(pkgJsonPath, JSON.stringify(pkgJson, null, 2))

    // 3. Modify vite.config.ts to add inspector plugin
    const viteConfigPath = path.join(tmpDir, 'vite.config.ts')
    let viteConfig = readFileSync(viteConfigPath, 'utf-8')

    // Add inspector import
    viteConfig = viteConfig.replace(
      'import componentTagger from "@moatless/tagger/vite";',
      'import componentTagger from "@moatless/tagger/vite";\nimport inspector from "@moatless/inspector/vite";',
    )

    // Add inspector() to plugins array
    viteConfig = viteConfig.replace('componentTagger()', 'componentTagger(), inspector()')

    writeFileSync(viteConfigPath, viteConfig)

    // 4. Install dependencies
    execSync('bun install', {
      cwd: tmpDir,
      stdio: 'pipe',
      timeout: 60_000,
    })

    // 5. Start dev server on random port
    devProcess = spawn('bunx', ['vite', '--port', String(port)], {
      cwd: tmpDir,
      stdio: 'pipe',
      env: { ...process.env, PORT: String(port) },
    })

    // 6. Wait for server to be ready
    await waitForServer(`http://localhost:${port}`, 30_000)
  }, 120_000)

  afterAll(() => {
    if (devProcess) {
      devProcess.kill()
    }
    if (tmpDir) {
      rmSync(tmpDir, { recursive: true, force: true })
    }
  })

  it('injects inspector script into HTML', async () => {
    const response = await fetch(`http://localhost:${port}/`)
    const html = await response.text()

    // Verify inspector script content is injected
    expect(html).toContain('INSPECTOR_SCRIPT_LOADED')
    expect(html).toContain('HEARTBEAT')
    expect(html).toContain('preview-iframe')
    // data-component-id is referenced in the inspector client code
    expect(html).toContain('data-component-id')
  })

  it('serves inspector script with correct serverId', async () => {
    const response = await fetch(`http://localhost:${port}/`)
    const html = await response.text()

    // Script should be in a <script> tag in body
    expect(html).toContain('<script>')
    // Default serverId is 'vite-dev'
    expect(html).toContain('vite-dev')
    // Placeholder should be replaced
    expect(html).not.toContain('__SERVER_ID__')
  })
})

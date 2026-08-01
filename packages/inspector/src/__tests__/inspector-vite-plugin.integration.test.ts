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

    // 5. Start dev server on random port (detached so the whole process group can be killed)
    devProcess = spawn('bunx', ['vite', '--port', String(port)], {
      cwd: tmpDir,
      stdio: 'pipe',
      detached: true,
      env: { ...process.env, PORT: String(port) },
    })

    // 6. Wait for server to be ready
    await waitForServer(`http://localhost:${port}`, 30_000)
  }, 120_000)

  afterAll(async () => {
    if (devProcess?.pid !== undefined && devProcess.exitCode === null) {
      // Kill the whole process group (bunx spawns vite/esbuild children that
      // keep writing to tmpDir if only the wrapper is killed)
      const exited = new Promise<void>((resolve) => {
        devProcess.once('exit', () => resolve())
      })
      try {
        process.kill(-devProcess.pid, 'SIGTERM')
      } catch {
        devProcess.kill()
      }
      const timeout = new Promise<'timeout'>((resolve) =>
        setTimeout(() => resolve('timeout'), 5000),
      )
      if ((await Promise.race([exited, timeout])) === 'timeout') {
        try {
          process.kill(-devProcess.pid, 'SIGKILL')
        } catch {
          devProcess.kill('SIGKILL')
        }
        await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 2000))])
      }
    }
    if (tmpDir) {
      rmSync(tmpDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 })
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

  it('injects the T3 annotation client alongside it', async () => {
    const response = await fetch(`http://localhost:${port}/`)
    const html = await response.text()

    expect(html).toContain('MoatlessPreviewAnnotation')
    expect(html).toContain('preview:element-picked')
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

import { exec } from 'node:child_process'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'

const execAsync = promisify(exec)

const HELLO_WORLD_REPO = 'https://github.com/aorwall/hello-world-ui.git'

export interface TestRepo {
  workspaceDir: string
  cleanup: () => Promise<void>
}

/**
 * Clone the hello-world-ui repository to a temporary directory
 */
export async function setupTestRepo(): Promise<TestRepo> {
  // Create temp directory
  const tempDir = await mkdtemp(join(tmpdir(), 'sandbox-test-'))
  const workspaceDir = join(tempDir, 'workspace')

  // Clone the repository
  await execAsync(`git clone --depth 1 ${HELLO_WORLD_REPO} ${workspaceDir}`)

  return {
    workspaceDir,
    cleanup: async () => {
      try {
        await rm(tempDir, { recursive: true, force: true })
      } catch {
        // Ignore cleanup errors
      }
    },
  }
}

/**
 * Create a minimal test repo without cloning (faster for basic tests)
 */
export async function setupMinimalTestRepo(): Promise<TestRepo> {
  const tempDir = await mkdtemp(join(tmpdir(), 'sandbox-test-'))
  const workspaceDir = join(tempDir, 'workspace')

  // Initialize a new git repo with minimal content
  await execAsync(`mkdir -p ${workspaceDir}`)
  await execAsync(`git init ${workspaceDir}`)
  await execAsync(
    `cd ${workspaceDir} && git config user.email "test@test.com" && git config user.name "Test"`,
  )

  // Create a simple index.html
  await execAsync(
    `echo '<!DOCTYPE html><html><body>Hello</body></html>' > ${workspaceDir}/index.html`,
  )

  // Create package.json for preview server
  const packageJson = JSON.stringify(
    {
      name: 'test-app',
      scripts: {
        start: 'npx http-server -p ${PORT:-3000}',
      },
    },
    null,
    2,
  )
  await execAsync(`echo '${packageJson}' > ${workspaceDir}/package.json`)

  // Initial commit
  await execAsync(`cd ${workspaceDir} && git add -A && git commit -m "Initial commit"`)

  return {
    workspaceDir,
    cleanup: async () => {
      try {
        await rm(tempDir, { recursive: true, force: true })
      } catch {
        // Ignore cleanup errors
      }
    },
  }
}

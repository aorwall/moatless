import { execSync } from 'node:child_process'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'

const HELLO_WORLD_REPO = 'https://github.com/aorwall/hello-world-ui.git'

/**
 * Docker sandbox test context
 */
export interface DockerSandboxContext {
  /** Root temp directory (for cleanup) */
  rootDir: string
  /** Workspace directory with the cloned repo */
  tempDir: string
  dataDir: string
  containerName: string
  /** Host port mapped to container HTTP port 8001 */
  hostPort: number
  /** Host port mapped to container gRPC port 50052 */
  grpcPort: number
}

/**
 * Check if Docker is available for testing
 */
export async function isDockerAvailable(): Promise<boolean> {
  try {
    execSync('docker info', { timeout: 5000, stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

/**
 * Check if the host platform is compatible with the sandbox image
 */
export function isPlatformCompatible(): boolean {
  try {
    const arch = os.arch()
    return arch === 'x64' || arch === 'arm64'
  } catch {
    return false
  }
}

/**
 * Check if Claude authentication is available for agent tests
 */
export function hasAnthropicKey(): boolean {
  return !!process.env['CLAUDE_CODE_OAUTH_TOKEN'] || !!process.env['ANTHROPIC_API_KEY']
}

/**
 * Get an available port by binding to port 0 and releasing
 */
async function getAvailablePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, () => {
      const addr = server.address()
      if (addr && typeof addr === 'object') {
        const port = addr.port
        server.close(() => resolve(port))
      } else {
        server.close(() => reject(new Error('Failed to get port')))
      }
    })
    server.on('error', reject)
  })
}

/**
 * Set up Docker sandbox test context by cloning the hello-world-ui repo
 */
export async function setupDockerSandbox(): Promise<DockerSandboxContext> {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'sandbox-docker-test-'))

  // Clone hello-world-ui repository into workspace subdirectory
  const workspaceDir = path.join(rootDir, 'workspace')
  execSync(`git clone --depth 1 ${HELLO_WORLD_REPO} ${workspaceDir}`, { stdio: 'ignore' })

  // Create data directory
  const dataDir = path.join(rootDir, 'data')
  fs.mkdirSync(dataDir, { recursive: true })

  const containerName = `sandbox-grpc-test-${Date.now()}`

  // Get two available ports: one for HTTP, one for gRPC
  const hostPort = await getAvailablePort()
  const grpcPort = await getAvailablePort()

  return {
    rootDir,
    tempDir: workspaceDir,
    dataDir,
    containerName,
    hostPort,
    grpcPort,
  }
}

/**
 * Get the sandbox Docker image to use
 */
export function getSandboxImage(): string {
  return process.env['SANDBOX_DOCKER_IMAGE'] || 'aorwall/moatless-sandbox:latest'
}

/**
 * Start Docker sandbox container with HTTP and gRPC ports exposed
 */
export async function startDockerSandbox(
  ctx: DockerSandboxContext,
  image = getSandboxImage(),
): Promise<void> {
  const { tempDir, dataDir, containerName, hostPort, grpcPort } = ctx

  let cmd = `docker create --name "${containerName}"`
  cmd += ` -v "${tempDir}:${tempDir}"`
  cmd += ` -v "${dataDir}:/opt/moatless/data"`
  cmd += ` -w "${tempDir}"`
  cmd += ` -p ${hostPort}:8001`
  cmd += ` -p ${grpcPort}:50052`
  cmd += ` -e "WORKSPACE_DIR=${tempDir}"`
  cmd += ` -e "DATA_DIR=/opt/moatless/data"`
  cmd += ` -e "LOG_LEVEL=info"`
  cmd += ` -e "TASK_ID=${containerName}"`

  // Add Claude token if available
  if (process.env['CLAUDE_CODE_OAUTH_TOKEN']) {
    cmd += ` -e "CLAUDE_CODE_OAUTH_TOKEN=${process.env['CLAUDE_CODE_OAUTH_TOKEN']}"`
  }

  // Add Anthropic API key if available
  if (process.env['ANTHROPIC_API_KEY']) {
    cmd += ` -e "ANTHROPIC_API_KEY=${process.env['ANTHROPIC_API_KEY']}"`
  }

  cmd += ` ${image}`

  // Create and start container
  execSync(cmd, { stdio: 'ignore' })
  execSync(`docker start ${containerName}`, { stdio: 'ignore' })

  // Wait for HTTP health check (gRPC starts in parallel and will be ready too)
  await waitForContainerHealth(hostPort, 60_000)
}

/**
 * Stop and remove Docker sandbox container
 */
export async function stopDockerSandbox(ctx: DockerSandboxContext): Promise<void> {
  const { containerName, rootDir } = ctx

  try {
    execSync(`docker stop ${containerName}`, { stdio: 'ignore', timeout: 10_000 })
  } catch {
    // Ignore
  }

  try {
    execSync(`docker rm ${containerName}`, { stdio: 'ignore', timeout: 5000 })
  } catch {
    // Ignore
  }

  if (rootDir && fs.existsSync(rootDir)) {
    fs.rmSync(rootDir, { recursive: true, force: true })
  }
}

/**
 * Wait for container to respond to HTTP health checks
 */
async function waitForContainerHealth(hostPort: number, timeoutMs: number): Promise<void> {
  const startTime = Date.now()
  const healthUrl = `http://localhost:${hostPort}/health`

  while (Date.now() - startTime < timeoutMs) {
    try {
      const response = await fetch(healthUrl, {
        signal: AbortSignal.timeout(5000),
      })
      if (response.ok) {
        return
      }
    } catch {
      // Container not ready yet
    }
    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  throw new Error(`Container health check timed out after ${timeoutMs}ms`)
}

/**
 * Execute a command inside the Docker container
 */
export function dockerExec(
  containerName: string,
  command: string,
  options: { timeout?: number; user?: string } = {},
): string {
  const { timeout = 120_000, user } = options
  const userFlag = user ? `-u ${user}` : ''
  try {
    return execSync(`docker exec ${userFlag} ${containerName} bash -c "${command}"`, {
      timeout,
      encoding: 'utf-8',
    })
  } catch (error) {
    const execError = error as { stdout?: string; stderr?: string; message: string }
    throw new Error(
      `docker exec failed: ${execError.message}\nstdout: ${execError.stdout}\nstderr: ${execError.stderr}`,
    )
  }
}

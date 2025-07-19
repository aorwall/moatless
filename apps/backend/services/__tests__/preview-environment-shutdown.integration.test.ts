import { spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type {
  CreateRepositoryRequest,
  CreateTaskRequest,
  RepositoryConfig,
  ServerStatus,
} from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'

/**
 * Integration test for Preview Environment Graceful Shutdown
 *
 * This test verifies that the graceful shutdown functionality:
 * - Stops all running preview environment servers
 * - Cleans up metadata files for successfully stopped servers
 * - Handles force kill for stubborn processes
 * - Works with real processes and file system operations
 */
describe('Preview Environment Graceful Shutdown', () => {
  let app: any
  let tempDir: string
  let createdRepositoryIds: string[] = []
  let createdTaskIds: string[] = []

  beforeEach(async () => {
    // Create a temporary directory for our test environment
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'preview-shutdown-test-'))

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)

    app = await build()
  })

  afterEach(async () => {
    // Cleanup tasks
    for (const taskId of createdTaskIds) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${taskId}`,
        })
      } catch (_e) {
        // Task might already be deleted
      }
    }

    // Cleanup repositories
    for (const repoId of createdRepositoryIds) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/repositories/${repoId}`,
        })
      } catch (_e) {
        // Repository might already be deleted
      }
    }

    // Clean up temp directory
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }

    // Reset arrays
    createdRepositoryIds = []
    createdTaskIds = []

    // No server cleanup needed
  })

  it('should gracefully shutdown all preview environment servers', async () => {
    // Create multiple server files for testing
    const serverCode1 = `
const http = require('http');
const port = process.argv[2] || 3000;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ server: 'server1', port, pid: process.pid }));
});

server.listen(port, () => {
  console.log(\`Server1 running on port \${port} with PID \${process.pid}\`);
});

process.on('SIGTERM', () => {
  console.log('Server1 received SIGTERM, shutting down gracefully');
  server.close(() => {
    console.log('Server1 closed');
    process.exit(0);
  });
});
`

    const serverCode2 = `
const http = require('http');
const port = process.argv[2] || 3001;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ server: 'server2', port, pid: process.pid }));
});

server.listen(port, () => {
  console.log(\`Server2 running on port \${port} with PID \${process.pid}\`);
});

process.on('SIGTERM', () => {
  console.log('Server2 received SIGTERM, shutting down gracefully');
  server.close(() => {
    console.log('Server2 closed');
    process.exit(0);
  });
});
`

    await fs.writeFile(path.join(tempDir, 'server1.js'), serverCode1)
    await fs.writeFile(path.join(tempDir, 'server2.js'), serverCode2)

    // Create environments and tasks
    const env1 = await createRepository('Test Repository 1')
    const env2 = await createRepository('Test Repository 2')

    const task1 = await createTask(env1.id)
    const task2 = await createTask(env2.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Start both servers
    const start1Response = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task1.taskId}/server/start`,
    })
    expect(start1Response.statusCode).toBe(200)
    const start1Result = JSON.parse(start1Response.body)

    const start2Response = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task2.taskId}/server/start`,
    })
    expect(start2Response.statusCode).toBe(200)
    const start2Result = JSON.parse(start2Response.body)

    console.log(`Started server1 PID: ${start1Result.pid}, URL: ${start1Result.url}`)
    console.log(`Started server2 PID: ${start2Result.pid}, URL: ${start2Result.url}`)

    // Wait for servers to be fully started
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Verify both servers are running
    await verifyServerRunning(start1Result.url, { server: 'server1' })
    await verifyServerRunning(start2Result.url, { server: 'server2' })

    // Verify preview environment metadata files exist
    const previewEnvDir = path.join(process.cwd(), 'data', 'preview-environments')
    const metadataBefore = await fs.readdir(previewEnvDir).catch(() => [])
    console.log('Metadata files before shutdown:', metadataBefore)

    expect(metadataBefore).toContain(`${task1.taskId}.json`)
    expect(metadataBefore).toContain(`${task2.taskId}.json`)

    // Call graceful shutdown
    console.log('Calling graceful shutdown...')
    await app.services.previewEnvironment.gracefulShutdown()

    // Wait for shutdown to complete
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Verify servers are stopped
    await verifyServerStopped(start1Result.url)
    await verifyServerStopped(start2Result.url)

    // Verify metadata files are cleaned up
    const metadataAfter = await fs.readdir(previewEnvDir).catch(() => [])
    console.log('Metadata files after shutdown:', metadataAfter)

    expect(metadataAfter).not.toContain(`${task1.taskId}.json`)
    expect(metadataAfter).not.toContain(`${task2.taskId}.json`)

    // Verify server status shows not running
    const status1 = await getServerStatus(task1.taskId)
    const status2 = await getServerStatus(task2.taskId)

    expect(status1.running).toBe(false)
    expect(status2.running).toBe(false)

    console.log('Graceful shutdown test completed successfully')
  }, 45000)

  it('should handle stubborn processes with force kill during shutdown', async () => {
    // Create a server that ignores SIGTERM (stubborn process)
    const stubbornServerCode = `
const http = require('http');
const port = process.argv[2] || 3000;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ server: 'stubborn', port, pid: process.pid }));
});

server.listen(port, () => {
  console.log(\`Stubborn server running on port \${port} with PID \${process.pid}\`);
});

// Ignore SIGTERM to simulate a stubborn process
process.on('SIGTERM', () => {
  console.log('Stubborn server received SIGTERM but ignoring it!');
  // Don't exit - be stubborn!
});
`

    await fs.writeFile(path.join(tempDir, 'stubborn-server.js'), stubbornServerCode)

    // Create environment and task
    const repo = await createRepository('Stubborn Test Environment')
    const task = await createTask(repo.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Start the stubborn server
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task.taskId}/server/start`,
    })
    expect(startResponse.statusCode).toBe(200)
    const startResult = JSON.parse(startResponse.body)

    console.log(`Started stubborn server PID: ${startResult.pid}, URL: ${startResult.url}`)

    // Wait for server to be fully started
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Verify server is running
    await verifyServerRunning(startResult.url, { server: 'stubborn' })

    // Call graceful shutdown (should force kill the stubborn process)
    console.log('Calling graceful shutdown on stubborn process...')
    await app.services.previewEnvironment.gracefulShutdown()

    // Wait for shutdown to complete
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Verify server is stopped (should be force killed)
    await verifyServerStopped(startResult.url)

    // Verify metadata file is cleaned up
    const previewEnvDir = path.join(process.cwd(), 'data', 'preview-environments')
    const metadataAfter = await fs.readdir(previewEnvDir).catch(() => [])

    expect(metadataAfter).not.toContain(`${task.taskId}.json`)

    // Verify server status shows not running
    const status = await getServerStatus(task.taskId)
    expect(status.running).toBe(false)

    console.log('Stubborn process force kill test completed successfully')
  }, 45000)

  it('should handle shutdown with orphaned metadata files', async () => {
    // Create server
    const serverCode = `
const http = require('http');
const port = process.argv[2] || 3000;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ server: 'orphan-test', port, pid: process.pid }));
});

server.listen(port, () => {
  console.log(\`Orphan test server running on port \${port} with PID \${process.pid}\`);
});

process.on('SIGTERM', () => {
  console.log('Orphan test server received SIGTERM, shutting down');
  server.close(() => {
    process.exit(0);
  });
});
`

    await fs.writeFile(path.join(tempDir, 'orphan-test-server.js'), serverCode)

    // Create environment and task
    const repo = await createRepository('Orphan Test Environment')
    const task = await createTask(repo.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Start server
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task.taskId}/server/start`,
    })
    expect(startResponse.statusCode).toBe(200)
    const startResult = JSON.parse(startResponse.body)

    console.log(`Started orphan test server PID: ${startResult.pid}`)

    // Manually kill the process to simulate it being terminated externally
    // but leaving the metadata file behind
    await new Promise((resolve) => setTimeout(resolve, 1000))

    try {
      process.kill(startResult.pid, 'SIGKILL')
      console.log('Manually killed process to create orphaned metadata')
    } catch (_e) {
      console.log('Process might already be terminated')
    }

    // Wait for process to be fully killed
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Verify metadata file still exists (orphaned)
    const previewEnvDir = path.join(process.cwd(), 'data', 'preview-environments')
    const metadataBefore = await fs.readdir(previewEnvDir).catch(() => [])
    console.log('Metadata files before shutdown (should include orphaned):', metadataBefore)

    expect(metadataBefore).toContain(`${task.taskId}.json`)

    // Call graceful shutdown (should clean up orphaned metadata)
    console.log('Calling graceful shutdown to clean up orphaned metadata...')
    await app.services.previewEnvironment.gracefulShutdown()

    // Wait for cleanup to complete
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Verify orphaned metadata file is cleaned up
    const metadataAfter = await fs.readdir(previewEnvDir).catch(() => [])
    console.log('Metadata files after shutdown (should be cleaned):', metadataAfter)

    expect(metadataAfter).not.toContain(`${task.taskId}.json`)

    console.log('Orphaned metadata cleanup test completed successfully')
  }, 45000)

  it('should integrate with process signal handlers', async () => {
    // Create a simple server
    const serverCode = `
const http = require('http');
const port = process.argv[2] || 3000;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/plain' });
  res.end('Signal test server');
});

server.listen(port, () => {
  console.log(\`Signal test server running on port \${port}\`);
});

process.on('SIGTERM', () => {
  console.log('Signal test server received SIGTERM');
  server.close(() => {
    process.exit(0);
  });
});
`

    await fs.writeFile(path.join(tempDir, 'signal-test-server.js'), serverCode)

    // Create environment and task
    const repo = await createRepository('Signal Test Environment')
    const task = await createTask(repo.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Start server
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task.taskId}/server/start`,
    })
    expect(startResponse.statusCode).toBe(200)
    const startResult = JSON.parse(startResponse.body)

    console.log(`Started signal test server PID: ${startResult.pid}`)

    // Wait for server to be ready
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Test graceful shutdown triggered by the service directly
    console.log('Testing graceful shutdown integration...')

    // Verify server is running before shutdown
    const statusBefore = await getServerStatus(task.taskId)
    expect(statusBefore.running).toBe(true)

    // Call graceful shutdown (simulating what happens on SIGTERM/SIGINT)
    await app.services.previewEnvironment.gracefulShutdown()

    // Wait for shutdown to complete
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Verify server is stopped
    const statusAfter = await getServerStatus(task.taskId)
    expect(statusAfter.running).toBe(false)

    // Verify no metadata files remain
    const previewEnvDir = path.join(process.cwd(), 'data', 'preview-environments')
    const metadataAfter = await fs.readdir(previewEnvDir).catch(() => [])
    expect(metadataAfter).not.toContain(`${task.taskId}.json`)

    console.log('Signal handler integration test completed successfully')
  }, 30000)

  // Helper functions
  async function createRepository(name: string): Promise<RepositoryConfig> {
    const request: CreateRepositoryRequest = {
      name,
      description: 'Test repository for shutdown testing',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        setupCommands: ['echo "Environment ready"'],
      },
    }

    const response = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: request,
    })

    expect(response.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(response.body)
    createdRepositoryIds.push(repository.id)
    return repository
  }

  async function createTask(repositoryId: string): Promise<{ taskId: string }> {
    const request: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId,
      start: false, // Don't start agent
    }

    const response = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: request,
    })

    expect(response.statusCode).toBe(200)
    const result = JSON.parse(response.body)
    createdTaskIds.push(result.taskId)
    return result
  }

  async function getServerStatus(taskId: string): Promise<ServerStatus> {
    const response = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${taskId}/server/status`,
    })

    expect(response.statusCode).toBe(200)
    return JSON.parse(response.body)
  }

  async function verifyServerRunning(url: string, expectedData: unknown): Promise<void> {
    const { default: http } = await import('node:http')
    return new Promise((resolve, reject) => {
      const req = http.get(url, (res) => {
        let data = ''
        res.on('data', (chunk) => {
          data += chunk
        })
        res.on('end', () => {
          try {
            const parsed = JSON.parse(data)
            expect(parsed).toMatchObject(expectedData as any)
            resolve()
          } catch (error) {
            reject(new Error(`Server response validation failed: ${error}`))
          }
        })
      })
      req.on('error', (error) => {
        reject(new Error(`Failed to connect to server: ${error.message}`))
      })
      req.setTimeout(5000, () => {
        req.destroy()
        reject(new Error('Connection timeout'))
      })
    })
  }

  async function verifyServerStopped(url: string): Promise<void> {
    const { default: http } = await import('node:http')
    return new Promise((resolve, reject) => {
      const req = http.get(url, () => {
        reject(new Error('Server is still responding - should be stopped'))
      })
      req.on('error', () => {
        // This is expected - server should be stopped
        resolve()
      })
      req.setTimeout(2000, () => {
        req.destroy()
        resolve() // Timeout is also expected for a stopped server
      })
    })
  }
})

async function setupTempGitRepo(tempDir: string): Promise<void> {
  // Initialize git repository
  await runCommand('git', ['init'], tempDir)
  await runCommand('git', ['config', 'user.name', 'Test User'], tempDir)
  await runCommand('git', ['config', 'user.email', 'test@example.com'], tempDir)

  // Create initial README
  const readmePath = path.join(tempDir, 'README.md')
  await fs.writeFile(
    readmePath,
    '# Test Repository\n\nThis is a test repository for preview environment shutdown testing.\n',
  )

  // Add and commit initial file
  await runCommand('git', ['add', 'README.md'], tempDir)
  await runCommand('git', ['commit', '-m', 'Initial commit'], tempDir)
}

async function runCommand(command: string, args: string[], cwd: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      stdio: 'pipe',
      env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
    })

    let stderr = ''
    child.stderr?.on('data', (data: Buffer) => {
      stderr += data.toString()
    })

    child.on('close', (code: number) => {
      if (code !== 0) {
        reject(
          new Error(`Command ${command} ${args.join(' ')} failed with code ${code}: ${stderr}`),
        )
      } else {
        resolve()
      }
    })

    child.on('error', reject)
  })
}

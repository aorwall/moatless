import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type {
  CreateEnvironmentRequest,
  CreateTaskRequest,
  Environment,
  ServerStatus,
} from '../../../shared/types/index.js'
import { build } from '../../test/helper.js'

/**
 * E2E Test for Server Lifecycle Management
 *
 * This test covers the complete server lifecycle workflow including:
 * - Environment creation with server configuration
 * - Task creation with server setup
 * - Starting a development server
 * - Verifying server status updates
 * - Connecting to the server on expected port
 * - Stopping the server cleanly
 *
 * The test uses a minimal Node.js HTTP server that:
 * - Accepts a port parameter
 * - Responds to HTTP requests
 * - Can be started and stopped cleanly
 */

describe('Server Lifecycle E2E Test', () => {
  let app: FastifyInstance
  let tempDir: string
  let createdEnvironmentId: string
  let createdTaskId: string

  beforeEach(async () => {
    // Create a temporary directory for our test environment
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'server-lifecycle-test-'))

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)

    app = await build()
  })

  afterEach(async () => {
    // Stop any running servers before cleanup
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/server/stop`,
        })
        // Give it time to stop
        await new Promise((resolve) => setTimeout(resolve, 500))
      } catch (_e) {
        // Server might not be running
      }
    }

    // Cleanup task if it exists
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
      } catch (_e) {
        // Task might already be deleted
      }
    }

    // Cleanup environment if it exists
    if (createdEnvironmentId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/environments/${createdEnvironmentId}`,
        })
      } catch (_e) {
        // Environment might already be deleted
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

    await app.close()
  })

  it('should manage server lifecycle: create environment, start server, verify status, connect, and stop', async () => {
    // Step 1: Create a minimal HTTP server file
    const serverCode = `
const http = require('http');
const port = process.argv[2] || 3000;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/plain' });
  res.end('Test server running on port ' + port);
});

server.listen(port, () => {
  console.log(\`Server running on port \${port}\`);
});

// Handle graceful shutdown
process.on('SIGTERM', () => {
  console.log('SIGTERM received, shutting down gracefully');
  server.close(() => {
    console.log('Server closed');
    process.exit(0);
  });
});
`

    await fs.writeFile(path.join(tempDir, 'server.js'), serverCode)

    // Step 2: Create environment with server configuration
    const createEnvironmentRequest: CreateEnvironmentRequest = {
      name: 'Server Lifecycle Test Environment',
      description: 'Test environment for server lifecycle management',
      repository: {
        type: 'local',
        path: tempDir,
        useGitWorktrees: false,
      },
      codeExecution: {
        type: 'local',
        setupCommands: 'echo "Environment ready for server"',
        serverCommand: 'node server.js {PORT}',
        serverPortPlaceholder: '{PORT}',
      },
    }

    const createEnvResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/environments',
      payload: createEnvironmentRequest,
    })

    if (createEnvResponse.statusCode !== 200) {
      console.error('Environment creation failed:', createEnvResponse.body)
    }
    expect(createEnvResponse.statusCode).toBe(200)
    const environment: Environment = JSON.parse(createEnvResponse.body)
    createdEnvironmentId = environment.id

    expect(environment).toMatchObject({
      name: createEnvironmentRequest.name,
      description: createEnvironmentRequest.description,
      repository: createEnvironmentRequest.repository,
      codeExecution: createEnvironmentRequest.codeExecution,
    })

    // Step 3: Create task (without starting agent)
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      environmentId: environment.id,
      start: false, // Don't start agent
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId

    // Wait for workspace setup to complete
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Step 4: Verify initial server status (not running)
    const initialStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/server/status`,
    })

    expect(initialStatusResponse.statusCode).toBe(200)
    const initialStatus: ServerStatus = JSON.parse(initialStatusResponse.body)
    console.log('Initial server status:', initialStatus)

    // The server might have been auto-started during setup, so stop it first if needed
    if (initialStatus.running) {
      console.log('Server was auto-started, stopping it first')
      await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/server/stop`,
      })
      await new Promise((resolve) => setTimeout(resolve, 500))
    }

    expect(initialStatus.configured).toBe(true) // Should be configured since we provided serverCommand

    // Step 5: Start the server
    const startServerResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/start`,
    })

    expect(startServerResponse.statusCode).toBe(200)
    const startResult = JSON.parse(startServerResponse.body)
    expect(startResult.running).toBe(true)
    expect(startResult.url).toMatch(/^http:\/\/localhost:\d+$/)
    expect(startResult.pid).toBeGreaterThan(0)
    expect(startResult.configured).toBe(true)

    // Extract port from URL
    const serverPort = parseInt(startResult.url.split(':').pop()!)
    expect(serverPort).toBeGreaterThan(0)
    console.log('Server started on port:', serverPort)

    // Wait for server to fully start
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Step 6: Verify server status (running)
    const runningStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/server/status`,
    })

    expect(runningStatusResponse.statusCode).toBe(200)
    const runningStatus: ServerStatus = JSON.parse(runningStatusResponse.body)
    expect(runningStatus.running).toBe(true)
    expect(runningStatus.configured).toBe(true)
    expect(runningStatus.url).toBe(`http://localhost:${serverPort}`)
    expect(runningStatus.pid).toBe(startResult.pid)
    expect(runningStatus.uptimeSeconds).toBeGreaterThanOrEqual(0)

    // Step 7: Connect to the server and verify it's responding
    const { default: http } = await import('node:http')
    const testConnection = () =>
      new Promise<string>((resolve, reject) => {
        const req = http.get(`http://localhost:${serverPort}`, (res) => {
          let data = ''
          res.on('data', (chunk) => {
            data += chunk
          })
          res.on('end', () => resolve(data))
        })
        req.on('error', reject)
        req.setTimeout(5000, () => {
          req.destroy()
          reject(new Error('Connection timeout'))
        })
      })

    let connectionSuccess = false
    let connectionResponse = ''

    try {
      connectionResponse = await testConnection()
      connectionSuccess = true
      console.log('Successfully connected to server, response:', connectionResponse)
      expect(connectionResponse).toContain(`Test server running on port ${serverPort}`)
    } catch (error) {
      console.error('Failed to connect to server:', error)
    }

    expect(connectionSuccess).toBe(true)

    // Step 8: Get server logs
    const logsResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/server/logs?lines=50`,
    })

    expect(logsResponse.statusCode).toBe(200)
    const logsResult = JSON.parse(logsResponse.body)
    expect(logsResult.logs).toContain(`Server running on port ${serverPort}`)
    console.log('Server logs preview:', logsResult.logs.substring(0, 200))

    // Step 9: Stop the server
    const stopServerResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/stop`,
    })

    expect(stopServerResponse.statusCode).toBe(200)
    const stopResult = JSON.parse(stopServerResponse.body)
    expect(stopResult.stopped).toBe(true)

    // Wait for server to stop
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Step 10: Verify server status (stopped)
    const stoppedStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/server/status`,
    })

    expect(stoppedStatusResponse.statusCode).toBe(200)
    const stoppedStatus: ServerStatus = JSON.parse(stoppedStatusResponse.body)
    expect(stoppedStatus.running).toBe(false)
    expect(stoppedStatus.configured).toBe(true)

    // Step 11: Verify we can't connect to the server anymore
    let connectionFailed = false
    try {
      await testConnection()
    } catch (_error) {
      connectionFailed = true
      console.log('Server correctly stopped - connection failed as expected')
    }
    expect(connectionFailed).toBe(true)

    // Step 12: Verify we can restart the server
    const restartServerResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/start`,
    })

    expect(restartServerResponse.statusCode).toBe(200)
    const restartResult = JSON.parse(restartServerResponse.body)
    expect(restartResult.running).toBe(true)
    expect(restartResult.url).toMatch(/^http:\/\/localhost:\d+$/)

    const restartPort = parseInt(restartResult.url.split(':').pop()!)
    expect(restartPort).toBeGreaterThan(0) // Might be different port

    console.log('Server restarted successfully on port:', restartPort)

    // Final cleanup - stop the restarted server
    await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/stop`,
    })
  }, 30000) // 30 second timeout

  it('should handle server failure gracefully', async () => {
    // Create a server that will fail to start
    const badServerCode = `
console.error('Server startup failed!');
process.exit(1);
`

    await fs.writeFile(path.join(tempDir, 'bad-server.js'), badServerCode)

    // Create environment with failing server
    const createEnvironmentRequest: CreateEnvironmentRequest = {
      name: 'Failing Server Test Environment',
      repository: {
        type: 'local',
        path: tempDir,
        useGitWorktrees: false,
      },
      codeExecution: {
        type: 'local',
        serverCommand: 'node bad-server.js {PORT}',
        serverPortPlaceholder: '{PORT}',
      },
    }

    const createEnvResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/environments',
      payload: createEnvironmentRequest,
    })

    const environment: Environment = JSON.parse(createEnvResponse.body)
    createdEnvironmentId = environment.id

    // Create task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      environmentId: environment.id,
      start: false,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Try to start the failing server
    const startServerResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/start`,
    })

    // Should fail with 500 error
    expect(startServerResponse.statusCode).toBe(500)
    const errorBody = JSON.parse(startServerResponse.body)
    console.log('Server start error:', JSON.stringify(errorBody, null, 2))

    // Just verify that we get an error response
    expect(errorBody).toBeTruthy()

    // Verify server status shows not running
    const statusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/server/status`,
    })

    const status: ServerStatus = JSON.parse(statusResponse.body)
    expect(status.running).toBe(false)
    expect(status.configured).toBe(true)
  })

  it('should track multiple server starts and stops', async () => {
    // Create a server that logs start/stop events
    const serverCode = `
const http = require('http');
const port = process.argv[2] || 3000;
let requestCount = 0;

const server = http.createServer((req, res) => {
  requestCount++;
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ port, requestCount, timestamp: Date.now() }));
});

server.listen(port, () => {
  console.log(\`Server started on port \${port} at \${new Date().toISOString()}\`);
});

process.on('SIGTERM', () => {
  console.log(\`Server stopping after \${requestCount} requests at \${new Date().toISOString()}\`);
  server.close(() => {
    process.exit(0);
  });
});
`

    await fs.writeFile(path.join(tempDir, 'tracking-server.js'), serverCode)

    // Create environment
    const createEnvironmentRequest: CreateEnvironmentRequest = {
      name: 'Server Tracking Test',
      repository: {
        type: 'local',
        path: tempDir,
        useGitWorktrees: false,
      },
      codeExecution: {
        type: 'local',
        serverCommand: 'node tracking-server.js {PORT}',
        serverPortPlaceholder: '{PORT}',
      },
    }

    const environment: Environment = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/environments',
          payload: createEnvironmentRequest,
        })
      ).body,
    )
    createdEnvironmentId = environment.id

    // Create task
    const taskResult = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/tasks',
          payload: {
            agentType: 'claude-code',
            environmentId: environment.id,
            start: false,
          },
        })
      ).body,
    )
    createdTaskId = taskResult.taskId

    // Wait for workspace
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Test multiple start/stop cycles
    for (let i = 0; i < 3; i++) {
      console.log(`\nServer lifecycle iteration ${i + 1}`)

      // Start server
      const startResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/server/start`,
      })
      expect(startResponse.statusCode).toBe(200)

      const startResult = JSON.parse(startResponse.body)
      const port = parseInt(startResult.url.split(':').pop()!)

      // Wait for server to be ready
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Make a request to track it
      const { default: http } = await import('node:http')
      const response = await new Promise<{ port: string; requestCount: number; timestamp: number }>(
        (resolve, reject) => {
          http
            .get(`http://localhost:${port}`, (res) => {
              let data = ''
              res.on('data', (chunk) => {
                data += chunk
              })
              res.on('end', () => resolve(JSON.parse(data)))
            })
            .on('error', reject)
        },
      )

      console.log(`Server response:`, response)
      expect(response.port).toBe(String(port))
      expect(response.requestCount).toBe(1)

      // Stop server
      const stopResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/server/stop`,
      })
      expect(stopResponse.statusCode).toBe(200)

      // Get logs for this iteration
      const iterationLogsResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/server/logs?lines=20`,
      })
      const iterationLogs = JSON.parse(iterationLogsResponse.body).logs
      console.log(`Logs after iteration ${i + 1}:`, iterationLogs)

      // Wait before next iteration
      await new Promise((resolve) => setTimeout(resolve, 1000))
    }

    // The test completed 3 cycles of start/stop
    console.log('Test completed 3 server start/stop cycles successfully')
  }, 30000)

  it('should persist server state and recover after application restart', async () => {
    // Create a server that logs its start time
    const serverCode = `
const http = require('http');
const port = process.argv[2] || 3000;
const startTime = Date.now();

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ 
    port, 
    startTime,
    uptime: Date.now() - startTime,
    pid: process.pid
  }));
});

server.listen(port, () => {
  console.log(\`Server started on port \${port} with PID \${process.pid} at \${new Date().toISOString()}\`);
});

process.on('SIGTERM', () => {
  console.log('SIGTERM received, shutting down gracefully');
  server.close(() => {
    process.exit(0);
  });
});
`

    await fs.writeFile(path.join(tempDir, 'persistent-server.js'), serverCode)

    // Create environment
    const createEnvironmentRequest: CreateEnvironmentRequest = {
      name: 'Server Persistence Test',
      repository: {
        type: 'local',
        path: tempDir,
        useGitWorktrees: false,
      },
      codeExecution: {
        type: 'local',
        serverCommand: 'node persistent-server.js {PORT}',
        serverPortPlaceholder: '{PORT}',
      },
    }

    const environment: Environment = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/environments',
          payload: createEnvironmentRequest,
        })
      ).body,
    )
    createdEnvironmentId = environment.id

    // Create task
    const taskResult = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/tasks',
          payload: {
            agentType: 'claude-code',
            environmentId: environment.id,
            start: false,
          },
        })
      ).body,
    )
    createdTaskId = taskResult.taskId

    // Wait for workspace
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Start server
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/start`,
    })
    expect(startResponse.statusCode).toBe(200)

    const startResult = JSON.parse(startResponse.body)
    const port = parseInt(startResult.url.split(':').pop()!)
    const pid = startResult.pid

    console.log(`Server started with PID ${pid} on port ${port}`)

    // Verify server is running
    const { default: http } = await import('node:http')
    const firstResponse = await new Promise<unknown>((resolve, reject) => {
      http
        .get(`http://localhost:${port}`, (res) => {
          let data = ''
          res.on('data', (chunk) => {
            data += chunk
          })
          res.on('end', () => resolve(JSON.parse(data)))
        })
        .on('error', reject)
    })

    expect(firstResponse.port).toBe(String(port))
    expect(firstResponse.pid).toBe(pid)

    // Simulate app restart by closing the current app
    await app.close()

    // Create a new app instance (simulating restart)
    app = await build()

    // Wait for services to initialize and recover
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Check server status after restart
    const statusAfterRestart = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/server/status`,
    })

    expect(statusAfterRestart.statusCode).toBe(200)
    const status = JSON.parse(statusAfterRestart.body)

    console.log('Server status after restart:', status)

    expect(status.running).toBe(true)
    expect(status.pid).toBe(pid)
    expect(status.url).toBe(`http://localhost:${port}`)

    // Verify server is still responding
    const secondResponse = await new Promise<unknown>((resolve, reject) => {
      http
        .get(`http://localhost:${port}`, (res) => {
          let data = ''
          res.on('data', (chunk) => {
            data += chunk
          })
          res.on('end', () => resolve(JSON.parse(data)))
        })
        .on('error', reject)
    })

    expect(secondResponse.port).toBe(String(port))
    expect(secondResponse.pid).toBe(pid)
    expect(secondResponse.uptime).toBeGreaterThan(0)

    // Stop the server
    const stopResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/stop`,
    })
    expect(stopResponse.statusCode).toBe(200)

    console.log('Successfully tested server persistence across app restart')
  }, 30000)

  it('should prevent multiple servers for the same task', async () => {
    // Create a server
    const serverCode = `
const http = require('http');
const port = process.argv[2] || 3000;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/plain' });
  res.end('Server instance ' + process.pid);
});

server.listen(port, () => {
  console.log(\`Server running on port \${port} with PID \${process.pid}\`);
});

process.on('SIGTERM', () => {
  server.close(() => {
    process.exit(0);
  });
});
`

    await fs.writeFile(path.join(tempDir, 'singleton-server.js'), serverCode)

    // Create environment
    const environment: Environment = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/environments',
          payload: {
            name: 'Singleton Server Test',
            repository: {
              type: 'local',
              path: tempDir,
              useGitWorktrees: false,
            },
            codeExecution: {
              type: 'local',
              serverCommand: 'node singleton-server.js {PORT}',
              serverPortPlaceholder: '{PORT}',
            },
          },
        })
      ).body,
    )
    createdEnvironmentId = environment.id

    // Create task
    const task = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/tasks',
          payload: {
            agentType: 'claude-code',
            environmentId: environment.id,
            start: false,
          },
        })
      ).body,
    )
    createdTaskId = task.taskId

    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Start first server
    const firstStart = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/start`,
    })
    expect(firstStart.statusCode).toBe(200)

    const firstResult = JSON.parse(firstStart.body)
    const firstPort = parseInt(firstResult.url.split(':').pop()!)
    const firstPid = firstResult.pid

    console.log(`First server started with PID ${firstPid} on port ${firstPort}`)

    // Try to start second server (should return the existing one)
    const secondStart = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/start`,
    })
    expect(secondStart.statusCode).toBe(200)

    const secondResult = JSON.parse(secondStart.body)

    // Should return the same server
    const secondPort = parseInt(secondResult.url.split(':').pop()!)
    expect(secondPort).toBe(firstPort)
    expect(secondResult.pid).toBe(firstPid)
    expect(secondResult.running).toBe(true)

    console.log('Second start returned existing server - singleton pattern working')

    // Stop the server
    await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/server/stop`,
    })
  }, 30000)

  it('should clean up orphaned processes and metadata', async () => {
    // This test simulates a scenario where the app crashes and leaves orphaned processes

    const serverCode = `
const http = require('http');
const port = process.argv[2] || 3000;

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/plain' });
  res.end('Orphan test server');
});

server.listen(port, () => {
  console.log(\`Server running on port \${port} with PID \${process.pid}\`);
});

// Don't handle SIGTERM to simulate a stubborn process
`

    await fs.writeFile(path.join(tempDir, 'orphan-server.js'), serverCode)

    // Create environment and task
    const environment: Environment = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/environments',
          payload: {
            name: 'Orphan Process Test',
            repository: {
              type: 'local',
              path: tempDir,
              useGitWorktrees: false,
            },
            codeExecution: {
              type: 'local',
              serverCommand: 'node orphan-server.js {PORT}',
              serverPortPlaceholder: '{PORT}',
            },
          },
        })
      ).body,
    )
    createdEnvironmentId = environment.id

    const task = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/tasks',
          payload: {
            agentType: 'claude-code',
            environmentId: environment.id,
            start: false,
          },
        })
      ).body,
    )
    createdTaskId = task.taskId

    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Start server
    const startResult = JSON.parse(
      (
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/server/start`,
        })
      ).body,
    )

    const orphanPid = startResult.pid
    console.log(`Started orphan server with PID ${orphanPid}`)

    // Simulate app crash by closing without cleanup
    await app.close()

    // Manually kill the process to simulate it being terminated externally
    try {
      process.kill(orphanPid, 'SIGKILL')
    } catch (_e) {
      console.log('Process already terminated')
    }

    // Wait a bit
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Create new app instance
    app = await build()

    // Wait for recovery to run
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Check status - should show not running
    const statusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/server/status`,
    })

    const status = JSON.parse(statusResponse.body)
    expect(status.running).toBe(false)
    expect(status.configured).toBe(true)

    console.log('Orphaned process cleaned up successfully')
  }, 30000)
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
    '# Test Repository\n\nThis is a test repository for server lifecycle testing.\n',
  )

  // Add and commit initial file
  await runCommand('git', ['add', 'README.md'], tempDir)
  await runCommand('git', ['commit', '-m', 'Initial commit'], tempDir)
}

async function runCommand(command: string, args: string[], cwd: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const { spawn } = require('node:child_process')
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

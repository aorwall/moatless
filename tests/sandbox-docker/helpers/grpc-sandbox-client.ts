import * as grpc from '@grpc/grpc-js'
import * as protoLoader from '@grpc/proto-loader'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const PROTO_PATH = path.resolve(
  __dirname,
  '../../../crates/sandbox-proto/proto/sandbox/v1/sandbox.proto',
)

export interface GrpcSandboxClient {
  checkHealth(): Promise<{ healthy: boolean }>
  getStatus(): Promise<{ statusJson: string }>
  searchFiles(query: string, limit?: number): Promise<{ files: string[] }>
  executeAgent(
    content: Array<{ text?: { text: string }; image?: { url: string; mediaType: string } }>,
    model?: string,
    systemPrompt?: string,
  ): Promise<Record<string, never>>
  stopAgent(): Promise<Record<string, never>>
  notifyMessages(): Promise<Record<string, never>>
  getServerStatus(name: string): Promise<{ statusJson: string }>
  streamServerLogs(name: string, lines?: number): Promise<Array<{ line: string }>>
  close(): void
}

/**
 * Create a gRPC client for the sandbox SandboxService.
 * Dynamically loads the proto file so no codegen is needed.
 */
export function createGrpcSandboxClient(grpcPort: number): GrpcSandboxClient {
  const packageDef = protoLoader.loadSync(PROTO_PATH, {
    keepCase: false, // convert to camelCase
    longs: String,
    enums: String,
    defaults: true,
    oneofs: true,
  })

  const proto = grpc.loadPackageDefinition(packageDef) as any
  const client = new proto.sandbox.v1.SandboxService(
    `localhost:${grpcPort}`,
    grpc.credentials.createInsecure(),
  )

  // Promisify a unary RPC call
  function callUnary(method: string, request: Record<string, unknown> = {}): Promise<any> {
    return new Promise((resolve, reject) => {
      client[method](request, (err: grpc.ServiceError | null, response: any) => {
        if (err) {
          reject(err)
        } else {
          resolve(response)
        }
      })
    })
  }

  return {
    checkHealth() {
      return callUnary('checkHealth', {})
    },

    getStatus() {
      return callUnary('getStatus', {})
    },

    searchFiles(query: string, limit = 100) {
      return callUnary('searchFiles', { query, limit })
    },

    executeAgent(content, model, systemPrompt) {
      return callUnary('executeAgent', { content, model, systemPrompt })
    },

    stopAgent() {
      return callUnary('stopAgent', {})
    },

    notifyMessages() {
      return callUnary('notifyMessages', {})
    },

    getServerStatus(name: string) {
      return callUnary('getServerStatus', { name })
    },

    streamServerLogs(name: string, lines?: number): Promise<Array<{ line: string }>> {
      return new Promise((resolve, reject) => {
        const request: Record<string, unknown> = { name }
        if (lines !== undefined) {
          request['lines'] = lines
        }

        const stream = client.streamServerLogs(request)
        const logLines: Array<{ line: string }> = []

        stream.on('data', (data: { line: string }) => {
          logLines.push(data)
        })

        stream.on('end', () => {
          resolve(logLines)
        })

        stream.on('error', (err: grpc.ServiceError) => {
          // UNIMPLEMENTED or NOT_FOUND are expected in some tests
          reject(err)
        })
      })
    },

    close() {
      client.close()
    },
  }
}

/**
 * Wait for the gRPC server to be ready by retrying checkHealth.
 */
export async function waitForGrpcReady(
  grpcClient: GrpcSandboxClient,
  timeoutMs = 30000,
): Promise<void> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    try {
      const resp = await grpcClient.checkHealth()
      if (resp.healthy) {
        return
      }
    } catch {
      // Not ready yet
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`gRPC health check timed out after ${timeoutMs}ms`)
}

/**
 * MCP JSON-RPC types following the protocol specification
 */

export interface McpRequest {
  jsonrpc: '2.0'
  method: string
  params?: unknown
  id?: string | number | null
}

export interface McpResponse {
  jsonrpc: '2.0'
  id?: string | number | null | undefined
  result?: unknown
  error?: {
    code: number
    message: string
    data?: unknown
  }
}

export interface McpTool {
  name: string
  description: string
  inputSchema: {
    type: 'object'
    properties: Record<string, unknown>
    required?: string[]
  }
}

export interface McpToolResult {
  content: Array<{
    type: string
    text?: string
    data?: string
    mimeType?: string
  }>
  isError?: boolean
}

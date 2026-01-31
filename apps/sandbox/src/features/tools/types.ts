import type { z } from 'zod/v4'

export interface SandboxToolResult {
  text: string
  isError?: boolean
}

export interface SandboxTool {
  name: string
  description: string
  schema: Record<string, z.ZodType>
  execute: (args: Record<string, unknown>) => Promise<SandboxToolResult>
}

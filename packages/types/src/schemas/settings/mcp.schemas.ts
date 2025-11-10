import { z } from 'zod'

// Stdio MCP server configuration (command-based)
export const StdioMcpServerConfigSchema = z.object({
  type: z.literal('stdio'),
  command: z.string().min(1, 'Command is required'),
  args: z.array(z.string()).default([]),
  env: z.record(z.string(), z.string()).default({}),
})

// HTTP MCP server configuration
export const HttpMcpServerConfigSchema = z.object({
  type: z.literal('http'),
  url: z.url('Must be a valid URL'),
  headers: z.record(z.string(), z.string()).default({}),
})

// Union of stdio and HTTP MCP server configurations with backward compatibility
// Automatically adds type: 'stdio' to configs missing the type field
export const McpServerConfigSchema = z.preprocess(
  (val) => {
    // Backward compatibility: if no type field exists, assume stdio
    if (typeof val === 'object' && val !== null && !('type' in val)) {
      return { ...val, type: 'stdio' }
    }
    return val
  },
  z.discriminatedUnion('type', [StdioMcpServerConfigSchema, HttpMcpServerConfigSchema]),
)

// MCP settings schema - record of server name to configuration
export const McpSettingsSchema = z.object({
  mcpServers: z.record(z.string(), McpServerConfigSchema).default({}),
})

// Update request schema
export const UpdateMcpSettingsRequestSchema = z.object({
  mcpServers: z.record(z.string(), McpServerConfigSchema).nullable().optional(),
})

// Response schema - includes validation and defaults
export const McpSettingsResponseSchema = z.object({
  settings: McpSettingsSchema,
  defaults: McpSettingsSchema,
})

// Derived types
export type McpServerConfig = z.infer<typeof McpServerConfigSchema>
export type McpSettings = z.infer<typeof McpSettingsSchema>
export type UpdateMcpSettingsRequest = z.infer<typeof UpdateMcpSettingsRequestSchema>
export type McpSettingsResponse = z.infer<typeof McpSettingsResponseSchema>

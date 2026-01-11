/**
 * @moatless/sandbox-api-contract
 *
 * OpenAPI route definitions for the sandbox HTTP API.
 * Used by both the sandbox app (to implement routes) and
 * the main server (to generate typed clients).
 */

// Endpoint paths
export { SANDBOX_ENDPOINTS, type SandboxEndpoint } from './endpoints'

// All routes
export * from './routes'

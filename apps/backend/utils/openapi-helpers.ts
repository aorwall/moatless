/**
 * OpenAPI Route Helpers
 *
 * Utility functions to reduce boilerplate when defining OpenAPI routes with @hono/zod-openapi.
 * These helpers provide consistent error handling, request/response structures, and common patterns.
 */

import { createRoute } from '@hono/zod-openapi'
import { z } from 'zod'

// ============================================================================
// Standard Response Definitions
// ============================================================================

/**
 * Standard error responses used across all routes
 */
export const standardErrors = {
  400: { description: 'Bad Request - Invalid input' },
  404: { description: 'Not Found - Resource does not exist' },
  500: { description: 'Internal Server Error' },
} as const

/**
 * Standard error responses for routes that may return 401 Unauthorized
 */
export const standardErrorsWithAuth = {
  ...standardErrors,
  401: { description: 'Unauthorized - Authentication required' },
} as const

// ============================================================================
// Response Helpers
// ============================================================================

/**
 * Helper to create a JSON response definition
 *
 * @param schema - Zod schema for the response body
 * @param description - Description of the response
 * @param statusCode - HTTP status code (default: 200)
 * @returns Response configuration object
 *
 * @example
 * ```typescript
 * jsonResponse(TaskSchema, 'Task retrieved successfully', 200)
 * // Returns:
 * // {
 * //   200: {
 * //     content: { 'application/json': { schema: TaskSchema } },
 * //     description: 'Task retrieved successfully'
 * //   }
 * // }
 * ```
 */
export function jsonResponse<T extends z.ZodType>(
  schema: T,
  description: string,
  statusCode = 200,
) {
  return {
    [statusCode]: {
      content: {
        'application/json': {
          schema,
        },
      },
      description,
    },
  }
}

/**
 * Combine a success response with standard error responses
 *
 * @param successSchema - Zod schema for successful response
 * @param successDescription - Description of success response
 * @param statusCode - HTTP status code for success (default: 200)
 * @returns Combined responses object
 *
 * @example
 * ```typescript
 * withStandardErrors(TaskSchema, 'Task created successfully', 201)
 * // Returns responses object with 201, 400, 404, 500
 * ```
 */
export function withStandardErrors<T extends z.ZodType>(
  successSchema: T,
  successDescription: string,
  statusCode = 200,
) {
  return {
    ...jsonResponse(successSchema, successDescription, statusCode),
    ...standardErrors,
  }
}

/**
 * Create a 204 No Content response with standard errors
 *
 * @param description - Description of the operation
 * @returns Responses object with 204 and standard errors
 *
 * @example
 * ```typescript
 * noContentResponse('Resource deleted successfully')
 * ```
 */
export function noContentResponse(description: string) {
  return {
    204: { description },
    ...standardErrors,
  }
}

// ============================================================================
// Request Helpers
// ============================================================================

/**
 * Helper to create a JSON request body definition
 *
 * @param schema - Zod schema for the request body
 * @param description - Description of the request body (optional)
 * @returns Request body configuration
 *
 * @example
 * ```typescript
 * jsonBody(CreateTaskSchema, 'Task creation parameters')
 * // Returns:
 * // {
 * //   body: {
 * //     content: { 'application/json': { schema: CreateTaskSchema } },
 * //     description: 'Task creation parameters'
 * //   }
 * // }
 * ```
 */
export function jsonBody<T extends z.ZodType>(schema: T, description?: string) {
  return {
    body: {
      content: {
        'application/json': {
          schema,
        },
      },
      description: description || 'Request body',
    },
  }
}

// ============================================================================
// Common Path Parameter Schemas
// ============================================================================

/**
 * Pre-defined path parameter schemas for common resource identifiers
 *
 * @example
 * ```typescript
 * import { pathParams } from './openapi-helpers.js'
 *
 * // Use in route definition
 * request: { params: pathParams.taskId }
 * ```
 */
export const pathParams = {
  taskId: z.object({ taskId: z.string().min(1) }),
  repositoryId: z.object({ repositoryId: z.string().min(1) }),
  workspaceId: z.object({ workspaceId: z.string().min(1) }),
  commandId: z.object({ commandId: z.string().min(1) }),
  sessionId: z.object({ sessionId: z.string().min(1) }),
  uploadId: z.object({ uploadId: z.string().min(1) }),
  filename: z.object({ filename: z.string().min(1) }),
} as const

/**
 * Combine multiple parameter schemas
 *
 * @param params - Parameter schemas to combine
 * @returns Merged parameter schema
 *
 * @example
 * ```typescript
 * // For route like /tasks/:taskId/files/:filename
 * const params = combineParams(pathParams.taskId, pathParams.filename)
 * ```
 */
export function combineParams(...params: z.ZodObject<any>[]) {
  return params.reduce((acc, param) => acc.merge(param))
}

// ============================================================================
// CRUD Route Builders
// ============================================================================

/**
 * Create a POST route configuration (Create resource)
 *
 * @param path - Route path (e.g., '/api/v1/tasks')
 * @param bodySchema - Zod schema for request body
 * @param responseSchema - Zod schema for response
 * @param description - Description of the request body
 * @param statusCode - Success status code (default: 201 Created)
 * @returns Complete route configuration
 *
 * @example
 * ```typescript
 * const createTaskRoute = createRoute(
 *   createPostRoute(
 *     '/api/v1/tasks',
 *     CreateTaskRequestSchema,
 *     TaskResponseSchema,
 *     'Task creation parameters'
 *   )
 * )
 * ```
 */
export function createPostRoute<TBody extends z.ZodType, TResponse extends z.ZodType>(
  path: string,
  bodySchema: TBody,
  responseSchema: TResponse,
  description: string,
  statusCode = 201,
) {
  return createRoute({
    method: 'post',
    path,
    request: jsonBody(bodySchema, description),
    responses: withStandardErrors(responseSchema, 'Created successfully', statusCode),
  })
}

/**
 * Create a GET by ID route configuration (Read single resource)
 *
 * @param path - Route path with parameter (e.g., '/api/v1/tasks/{taskId}')
 * @param paramsSchema - Zod schema for path parameters
 * @param responseSchema - Zod schema for response
 * @param description - Description of the response
 * @returns Complete route configuration
 *
 * @example
 * ```typescript
 * const getTaskRoute = createRoute(
 *   createGetByIdRoute(
 *     '/api/v1/tasks/{taskId}',
 *     pathParams.taskId,
 *     TaskSchema,
 *     'Task retrieved successfully'
 *   )
 * )
 * ```
 */
export function createGetByIdRoute<TParams extends z.ZodObject<any>, TResponse extends z.ZodType>(
  path: string,
  paramsSchema: TParams,
  responseSchema: TResponse,
  description: string,
) {
  return createRoute({
    method: 'get',
    path,
    request: { params: paramsSchema },
    responses: withStandardErrors(responseSchema, description),
  })
}

/**
 * Create a GET list route configuration (Read list of resources)
 *
 * @param path - Route path (e.g., '/api/v1/tasks')
 * @param querySchema - Zod schema for query parameters (optional)
 * @param responseSchema - Zod schema for response
 * @param description - Description of the response
 * @returns Complete route configuration
 *
 * @example
 * ```typescript
 * const listTasksRoute = createRoute(
 *   createGetListRoute(
 *     '/api/v1/tasks',
 *     TaskListQuerySchema,
 *     TaskListResponseSchema,
 *     'Tasks retrieved successfully'
 *   )
 * )
 * ```
 */
export function createGetListRoute<TQuery extends z.ZodType, TResponse extends z.ZodType>(
  path: string,
  querySchema: TQuery | undefined,
  responseSchema: TResponse,
  description: string,
) {
  return createRoute({
    method: 'get',
    path,
    request: querySchema ? { query: querySchema } : {},
    responses: withStandardErrors(responseSchema, description),
  })
}

/**
 * Create a PUT route configuration (Update resource)
 *
 * @param path - Route path with parameter (e.g., '/api/v1/tasks/{taskId}')
 * @param paramsSchema - Zod schema for path parameters
 * @param bodySchema - Zod schema for request body
 * @param responseSchema - Zod schema for response
 * @param description - Description of the response
 * @returns Complete route configuration
 *
 * @example
 * ```typescript
 * const updateTaskRoute = createRoute(
 *   createPutRoute(
 *     '/api/v1/tasks/{taskId}',
 *     pathParams.taskId,
 *     UpdateTaskSchema,
 *     TaskSchema,
 *     'Task updated successfully'
 *   )
 * )
 * ```
 */
export function createPutRoute<
  TParams extends z.ZodObject<any>,
  TBody extends z.ZodType,
  TResponse extends z.ZodType,
>(
  path: string,
  paramsSchema: TParams,
  bodySchema: TBody,
  responseSchema: TResponse,
  description: string,
) {
  return createRoute({
    method: 'put',
    path,
    request: {
      params: paramsSchema,
      ...jsonBody(bodySchema, 'Update data'),
    },
    responses: withStandardErrors(responseSchema, description),
  })
}

/**
 * Create a DELETE route configuration (Delete resource)
 *
 * @param path - Route path with parameter (e.g., '/api/v1/tasks/{taskId}')
 * @param paramsSchema - Zod schema for path parameters
 * @param description - Description of the delete operation (optional)
 * @returns Complete route configuration
 *
 * @example
 * ```typescript
 * const deleteTaskRoute = createRoute(
 *   createDeleteRoute('/api/v1/tasks/{taskId}', pathParams.taskId)
 * )
 * ```
 */
export function createDeleteRoute<TParams extends z.ZodObject<any>>(
  path: string,
  paramsSchema: TParams,
  description?: string,
) {
  return createRoute({
    method: 'delete',
    path,
    request: { params: paramsSchema },
    responses: noContentResponse(description || 'Deleted successfully'),
  })
}

/**
 * Create a POST nested action route configuration (Action on sub-resource)
 *
 * @param path - Route path with parameters (e.g., '/api/v1/tasks/{taskId}/messages')
 * @param paramsSchema - Zod schema for path parameters
 * @param bodySchema - Zod schema for request body
 * @param responseSchema - Zod schema for response
 * @param bodyDescription - Description of the request body
 * @param responseDescription - Description of the response
 * @returns Complete route configuration
 *
 * @example
 * ```typescript
 * const sendMessageRoute = createRoute(
 *   createNestedActionRoute(
 *     '/api/v1/tasks/{taskId}/messages',
 *     pathParams.taskId,
 *     SendMessageSchema,
 *     MessageResponseSchema,
 *     'Message to send',
 *     'Message sent successfully'
 *   )
 * )
 * ```
 */
export function createNestedActionRoute<
  TParams extends z.ZodObject<any>,
  TBody extends z.ZodType,
  TResponse extends z.ZodType,
>(
  path: string,
  paramsSchema: TParams,
  bodySchema: TBody,
  responseSchema: TResponse,
  bodyDescription: string,
  responseDescription: string,
) {
  return createRoute({
    method: 'post',
    path,
    request: {
      params: paramsSchema,
      ...jsonBody(bodySchema, bodyDescription),
    },
    responses: withStandardErrors(responseSchema, responseDescription),
  })
}

/**
 * Create a GET nested resource route configuration (Read sub-resource)
 *
 * @param path - Route path with parameters (e.g., '/api/v1/tasks/{taskId}/commands')
 * @param paramsSchema - Zod schema for path parameters
 * @param responseSchema - Zod schema for response
 * @param description - Description of the response
 * @returns Complete route configuration
 *
 * @example
 * ```typescript
 * const getCommandsRoute = createRoute(
 *   createNestedGetRoute(
 *     '/api/v1/tasks/{taskId}/commands',
 *     pathParams.taskId,
 *     CommandsResponseSchema,
 *     'Commands retrieved successfully'
 *   )
 * )
 * ```
 */
export function createNestedGetRoute<TParams extends z.ZodObject<any>, TResponse extends z.ZodType>(
  path: string,
  paramsSchema: TParams,
  responseSchema: TResponse,
  description: string,
) {
  return createRoute({
    method: 'get',
    path,
    request: { params: paramsSchema },
    responses: withStandardErrors(responseSchema, description),
  })
}

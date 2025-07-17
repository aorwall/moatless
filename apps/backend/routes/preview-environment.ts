import {
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  type ServerStatus,
  ServerStatusSchema,
  ServerStopResponseSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { proxy } from 'hono/proxy'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'

export async function registerPreviewEnvironmentRoutes(app: Hono<AppBindings>, prefix: string) {
  // Start development server
  app.post(
    `${prefix}/:taskId/server/start`,
    describeRoute({
      description: 'Start development server for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server started successfully',
          content: {
            'application/json': {
              schema: resolver(ServerStatusSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const result: ServerStatus = await services.workspace.startServer(taskId)
      return c.json(result)
    },
  )

  // Stop development server
  app.post(
    `${prefix}/:taskId/server/stop`,
    describeRoute({
      description: 'Stop development server for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server stopped successfully',
          content: {
            'application/json': {
              schema: resolver(ServerStopResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const stopped = await services.workspace.stopServer(taskId)
      return c.json({
        stopped,
        runtime: 0, // TODO: Calculate actual runtime
      })
    },
  )

  // Restart development server
  app.post(
    `${prefix}/:taskId/server/restart`,
    describeRoute({
      description: 'Restart development server for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server restarted successfully',
          content: {
            'application/json': {
              schema: resolver(ServerStatusSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const result = await services.workspace.restartServer(taskId)
      return c.json(result)
    },
  )

  // Get server status
  app.get(
    `${prefix}/:taskId/server/status`,
    describeRoute({
      description: 'Get development server status for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server status',
          content: {
            'application/json': {
              schema: resolver(ServerStatusSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const status = await services.workspace.getServerStatus(taskId)
      return c.json(status)
    },
  )

  // Get server logs
  app.get(
    `${prefix}/:taskId/server/logs`,
    describeRoute({
      description: 'Get development server logs for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
        {
          name: 'lines',
          in: 'query',
          required: false,
          description: 'Number of log lines to return',
          schema: {
            type: 'number',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server logs',
          content: {
            'application/json': {
              schema: resolver(ServerLogsResponseExtendedSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      const queryParams = c.req.query()
      const validatedQuery = ServerLogsQuerySchema.parse(queryParams)
      const { lines } = validatedQuery

      // Task validation and capability check now handled in workspace service
      const logs = await services.workspace.getServerLogs(taskId, lines)
      return c.json({
        logs,
        lines: logs.split('\n').length,
        truncated: false, // TODO: Implement truncation logic
      })
    },
  )

  // Proxy requests to the preview server with script injection
  app.all(
    `${prefix}/:taskId/proxy/*`,
    describeRoute({
      description: 'Proxy requests to the preview server with script injection',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Proxied response',
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Task not found or server not running',
        },
        502: {
          description: 'Bad gateway - preview server error',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Check if server is running
      if (!(await services.workspace.canManageServer(taskId))) {
        throw new HTTPException(404, { message: 'Preview capability not available for this task' })
      }

      const serverStatus = await services.workspace.getServerStatus(taskId)
      if (!serverStatus.running) {
        throw new HTTPException(404, { message: 'Preview server not running' })
      }

      // Get the path after /proxy/
      const proxyPath = c.req.path.split(`/${taskId}/proxy/`)[1] || ''
      const queryString = c.req.url.split('?')[1] || ''
      const fullPath = queryString ? `/${proxyPath}?${queryString}` : `/${proxyPath}`

      try {
        // Get the workspace
        const workspace = await services.workspace.getOrCreateWorkspace(taskId)

        // Check if environment supports direct port access (local) or needs proxy (docker)
        const serverPort = workspace.getPreviewPort()

        if (serverPort) {
          // Local environment with direct port access
          const response = await proxy(`http://localhost:${serverPort}${fullPath}`, {
            headers: {
              ...c.req.header(),
              'X-Forwarded-For': '127.0.0.1',
              'X-Forwarded-Host': c.req.header('host'),
            },
          })

          // Check if it's an HTML response that we want to inject scripts into
          const contentType = response.headers.get('content-type') || ''
          if (contentType.includes('text/html')) {
            // Read the HTML content
            const html = await response.text()

            // Create the injection script
            const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
              
              // Send a message to parent to confirm script is loaded
              window.addEventListener('load', () => {
                if (window.parent && window.parent !== window) {
                  window.parent.postMessage({
                    type: 'INSPECTOR_SCRIPT_LOADED',
                    payload: { version: '1.0.0', taskId: '${taskId}' },
                    source: 'preview-iframe'
                  }, '*');
                }
              });
            </script>
          `

            // Inject the script before </head> or at the beginning of <body>
            let modifiedHtml = html
            if (html.includes('</head>')) {
              modifiedHtml = html.replace('</head>', `${injectedScript}</head>`)
            } else if (html.includes('<body>')) {
              modifiedHtml = html.replace('<body>', `<body>${injectedScript}`)
            } else {
              // Fallback: add at the beginning
              modifiedHtml = `${injectedScript}${html}`
            }

            return new Response(modifiedHtml, {
              status: response.status,
              headers: {
                ...Object.fromEntries(response.headers.entries()),
                'content-length': modifiedHtml.length.toString(),
              },
            })
          }

          // Return unmodified response for non-HTML content
          return response
        }
        // Docker environment - use proxy method
        const proxyResponse = await workspace.proxyPreviewRequest({
          method: c.req.method,
          path: fullPath,
          headers: c.req.header() as unknown as Headers,
          body: c.req.method !== 'GET' && c.req.method !== 'HEAD' ? await c.req.text() : undefined,
        })

        // Check if it's an HTML response that we want to inject scripts into
        const contentType = proxyResponse.headers.get('content-type') || ''
        if (contentType.includes('text/html')) {
          // Read the HTML content
          const html =
            typeof proxyResponse.body === 'string'
              ? proxyResponse.body
              : proxyResponse.body.toString('utf-8')

          // Create the injection script
          const injectedScript = `
              <script>
                console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
                
                // Send a message to parent to confirm script is loaded
                window.addEventListener('load', () => {
                  if (window.parent && window.parent !== window) {
                    window.parent.postMessage({
                      type: 'INSPECTOR_SCRIPT_LOADED',
                      payload: { version: '1.0.0', taskId: '${taskId}' },
                      source: 'preview-iframe'
                    }, '*');
                  }
                });
              </script>
            `

          // Inject the script before </head> or at the beginning of <body>
          let modifiedHtml = html
          if (html.includes('</head>')) {
            modifiedHtml = html.replace('</head>', `${injectedScript}</head>`)
          } else if (html.includes('<body>')) {
            modifiedHtml = html.replace('<body>', `<body>${injectedScript}`)
          } else {
            // Fallback: add at the beginning
            modifiedHtml = `${injectedScript}${html}`
          }

          return new Response(modifiedHtml, {
            status: proxyResponse.status,
            headers: proxyResponse.headers,
          })
        }

        // Return unmodified response for non-HTML content
        return new Response(proxyResponse.body, {
          status: proxyResponse.status,
          headers: proxyResponse.headers,
        })
      } catch (error) {
        if (error instanceof Error && error.message.includes('not running')) {
          throw new HTTPException(404, { message: 'Preview server not running' })
        }
        throw new HTTPException(502, { message: 'Error proxying request to preview server' })
      }
    },
  )
}

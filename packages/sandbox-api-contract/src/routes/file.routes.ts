import { createRoute } from '@hono/zod-openapi'
import {
  FileTreeRequestSchema,
  FileTreeWithChangesResponseSchema,
  ListFilesRequestSchema,
  ListFilesResponseSchema,
  ReadFileRequestSchema,
  ReadFileResponseSchema,
  ReadFileWithChangesRequestSchema,
  ReadFileWithChangesResponseSchema,
  SearchFilesRequestSchema,
  SearchFilesResponseSchema,
  WriteFileRequestSchema,
  WriteFileResponseSchema,
} from '@moatless/sandbox-schemas'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const readFileRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.READ_FILE,
  tags: ['File'],
  description: 'Read a file from the workspace',
  request: {
    body: {
      content: {
        'application/json': {
          schema: ReadFileRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'File content',
      content: {
        'application/json': {
          schema: ReadFileResponseSchema,
        },
      },
    },
    404: {
      description: 'File not found',
      content: {
        'application/json': {
          schema: ReadFileResponseSchema,
        },
      },
    },
  },
})

export const writeFileRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.WRITE_FILE,
  tags: ['File'],
  description: 'Write content to a file in the workspace',
  request: {
    body: {
      content: {
        'application/json': {
          schema: WriteFileRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'File written',
      content: {
        'application/json': {
          schema: WriteFileResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to write file',
      content: {
        'application/json': {
          schema: WriteFileResponseSchema,
        },
      },
    },
  },
})

export const listFilesRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.LIST_FILES,
  tags: ['File'],
  description: 'List files in the workspace directory',
  request: {
    body: {
      content: {
        'application/json': {
          schema: ListFilesRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'File list',
      content: {
        'application/json': {
          schema: ListFilesResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to list files',
      content: {
        'application/json': {
          schema: ListFilesResponseSchema,
        },
      },
    },
  },
})

export const searchFilesRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.SEARCH_FILES,
  tags: ['File'],
  description: 'Search for files in the workspace',
  request: {
    body: {
      content: {
        'application/json': {
          schema: SearchFilesRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Search results',
      content: {
        'application/json': {
          schema: SearchFilesResponseSchema,
        },
      },
    },
    500: {
      description: 'Search failed',
      content: {
        'application/json': {
          schema: SearchFilesResponseSchema,
        },
      },
    },
  },
})

export const readFileWithChangesRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.READ_FILE_WITH_CHANGES,
  tags: ['File'],
  description: 'Read a file with git change information',
  request: {
    body: {
      content: {
        'application/json': {
          schema: ReadFileWithChangesRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'File content with changes',
      content: {
        'application/json': {
          schema: ReadFileWithChangesResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to read file',
      content: {
        'application/json': {
          schema: ReadFileWithChangesResponseSchema,
        },
      },
    },
  },
})

export const fileTreeWithChangesRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.FILE_TREE_WITH_CHANGES,
  tags: ['File'],
  description: 'Get file tree with git change information',
  request: {
    body: {
      content: {
        'application/json': {
          schema: FileTreeRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'File tree with changes',
      content: {
        'application/json': {
          schema: FileTreeWithChangesResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to get file tree',
      content: {
        'application/json': {
          schema: FileTreeWithChangesResponseSchema,
        },
      },
    },
  },
})

import type { z } from 'zod'

export const standardErrors = {
  400: { description: 'Bad Request - Invalid input' },
  404: { description: 'Not Found - Resource does not exist' },
  500: { description: 'Internal Server Error' },
} as const

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

export function noContentResponse(description: string) {
  return {
    204: { description },
    ...standardErrors,
  }
}

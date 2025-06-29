export const ERROR_CODES = {
  // Task errors
  TASK_NOT_FOUND: 'TASK_NOT_FOUND',
  TASK_ALREADY_EXISTS: 'TASK_ALREADY_EXISTS',
  TASK_INVALID_STATE: 'TASK_INVALID_STATE',

  // Environment errors
  ENVIRONMENT_NOT_FOUND: 'ENVIRONMENT_NOT_FOUND',
  ENVIRONMENT_INVALID_CONFIG: 'ENVIRONMENT_INVALID_CONFIG',

  // Command errors
  COMMAND_NOT_FOUND: 'COMMAND_NOT_FOUND',

  // Workspace errors
  WORKSPACE_ERROR: 'WORKSPACE_ERROR',
  WORKSPACE_SETUP_FAILED: 'WORKSPACE_SETUP_FAILED',

  // Agent errors
  AGENT_NOT_FOUND: 'AGENT_NOT_FOUND',
  AGENT_EXECUTION_ERROR: 'AGENT_EXECUTION_ERROR',

  // Validation errors
  VALIDATION_ERROR: 'VALIDATION_ERROR',

  // Generic errors
  INTERNAL_SERVER_ERROR: 'INTERNAL_SERVER_ERROR',
  NOT_IMPLEMENTED: 'NOT_IMPLEMENTED',
} as const

export type ErrorCode = (typeof ERROR_CODES)[keyof typeof ERROR_CODES]

export class MoatlessError extends Error {
  constructor(
    message: string,
    public code: ErrorCode,
    public statusCode: number = 500,
    public details?: unknown,
  ) {
    super(message)
    this.name = 'MoatlessError'
  }
}

export class TaskNotFoundError extends MoatlessError {
  constructor(taskId: string) {
    super(`Task ${taskId} not found`, ERROR_CODES.TASK_NOT_FOUND, 404)
  }
}

export class EnvironmentNotFoundError extends MoatlessError {
  constructor(environmentId: string) {
    super(`Environment ${environmentId} not found`, ERROR_CODES.ENVIRONMENT_NOT_FOUND, 404)
  }
}

export class CommandNotFoundError extends MoatlessError {
  constructor(commandName: string) {
    super(`Command ${commandName} not found`, ERROR_CODES.COMMAND_NOT_FOUND, 404)
  }
}

export class WorkspaceError extends MoatlessError {
  constructor(message: string, taskId: string, statusCode = 400) {
    super(message, ERROR_CODES.WORKSPACE_ERROR, statusCode, { taskId })
  }
}

export class ValidationError extends MoatlessError {
  constructor(message: string, errors: unknown) {
    super(message, ERROR_CODES.VALIDATION_ERROR, 400, { errors })
  }
}

export class CriticalError extends MoatlessError {
  constructor(
    message: string,
    public readonly taskId: string,
    public readonly iteration?: unknown,
  ) {
    super(message, ERROR_CODES.AGENT_EXECUTION_ERROR, 500, { taskId, iteration })
    this.name = 'CriticalError'
  }
}

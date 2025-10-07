import type { UseMutationResult } from '@tanstack/react-query'
import { vi } from 'vitest'

/**
 * Creates a mock mutation result with all required properties from React Query's UseMutationResult
 */
export function createMockMutationResult<
  TData = unknown,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  overrides: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {},
): UseMutationResult<TData, TError, TVariables, TContext> {
  const base: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {
    data: undefined,
    error: null,
    isError: false,
    isIdle: true,
    isPending: false,
    isPaused: false,
    isSuccess: false,
    failureCount: 0,
    failureReason: null,
    mutate: vi.fn(),
    mutateAsync: vi.fn(),
    reset: vi.fn(),
    status: 'idle',
    variables: undefined,
    submittedAt: 0,
    context: undefined,
  }

  return { ...base, ...overrides } as UseMutationResult<TData, TError, TVariables, TContext>
}

/**
 * Creates a mock mutation in a pending state
 */
export function createPendingMutation<
  TData = unknown,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  overrides: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {},
): UseMutationResult<TData, TError, TVariables, TContext> {
  const base: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {
    data: undefined,
    error: null,
    isError: false,
    isIdle: false,
    isPending: true,
    isSuccess: false,
    status: 'pending',
    variables: undefined,
    context: undefined,
    failureCount: 0,
    failureReason: null,
    isPaused: false,
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    reset: vi.fn(),
    submittedAt: Date.now(),
  }

  return {
    ...base,
    ...overrides,
  } as UseMutationResult<TData, TError, TVariables, TContext>
}

/**
 * Creates a mock mutation in a success state
 */
export function createSuccessMutation<
  TData = unknown,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  data: TData,
  overrides: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {},
): UseMutationResult<TData, TError, TVariables, TContext> {
  const base: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {
    data,
    error: null,
    isError: false,
    isIdle: false,
    isPending: false,
    isSuccess: true,
    status: 'success',
    variables: undefined,
    context: undefined,
    failureCount: 0,
    failureReason: null,
    isPaused: false,
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(data),
    reset: vi.fn(),
    submittedAt: Date.now(),
  }

  return {
    ...base,
    ...overrides,
  } as UseMutationResult<TData, TError, TVariables, TContext>
}

/**
 * Creates a mock mutation in an error state
 */
export function createErrorMutation<
  TData = unknown,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  error: TError,
  overrides: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {},
): UseMutationResult<TData, TError, TVariables, TContext> {
  const base: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {
    data: undefined,
    error,
    isError: true,
    isIdle: false,
    isPending: false,
    isSuccess: false,
    status: 'error',
    variables: undefined,
    context: undefined,
    failureCount: 1,
    failureReason: error,
    isPaused: false,
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockRejectedValue(error),
    reset: vi.fn(),
    submittedAt: Date.now(),
  }

  return {
    ...base,
    ...overrides,
  } as UseMutationResult<TData, TError, TVariables, TContext>
}

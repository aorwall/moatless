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
  return createMockMutationResult<TData, TError, TVariables, TContext>({
    isIdle: false,
    isPending: true,
    status: 'pending',
    ...overrides,
  })
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
  return createMockMutationResult<TData, TError, TVariables, TContext>({
    data,
    isIdle: false,
    isSuccess: true,
    status: 'success',
    ...overrides,
  })
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
  return createMockMutationResult<TData, TError, TVariables, TContext>({
    error,
    isIdle: false,
    isError: true,
    status: 'error',
    failureCount: 1,
    failureReason: error,
    ...overrides,
  })
}

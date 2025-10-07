import type { UseMutationResult, UseQueryResult } from '@tanstack/react-query'
import { vi } from 'vitest'

/**
 * Standardized API hook mocks for consistent testing
 */

/**
 * Creates a mock query hook with standard return values
 * @param data - The data to return
 * @param overrides - Optional overrides for the query result
 * @returns Mock query result
 */
export function createMockQuery<TData = unknown, TError = Error>(
  data: TData | undefined = undefined,
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  const hasData = data !== undefined

  const baseResult: Partial<UseQueryResult<TData, TError>> = {
    data,
    error: null,
    isError: false,
    isLoading: false,
    isLoadingError: false,
    isRefetchError: false,
    isSuccess: hasData,
    isPending: !hasData,
    status: hasData ? 'success' : 'pending',
    dataUpdatedAt: hasData ? Date.now() : 0,
    errorUpdatedAt: 0,
    failureCount: 0,
    failureReason: null,
    errorUpdateCount: 0,
    isFetched: hasData,
    isFetchedAfterMount: hasData,
    isFetching: false,
    isInitialLoading: false,
    isPaused: false,
    isPlaceholderData: false,
    isRefetching: false,
    isStale: false,
    refetch: vi.fn().mockResolvedValue({
      data,
      error: null,
      isError: false,
      isSuccess: true,
    }),
    fetchStatus: 'idle',
    promise: Promise.resolve(data),
  }

  return {
    ...baseResult,
    ...overrides,
  } as UseQueryResult<TData, TError>
}

/**
 * Creates a mock query hook in loading state
 */
export function createMockLoadingQuery<TData = unknown, TError = Error>(
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  return createMockQuery<TData, TError>(undefined, {
    isLoading: true,
    isInitialLoading: true,
    isFetching: true,
    isPending: true,
    isSuccess: false,
    status: 'pending',
    fetchStatus: 'fetching',
    ...overrides,
  })
}

/**
 * Creates a mock query hook in error state
 */
export function createMockErrorQuery<TData = unknown, TError = Error>(
  error: TError,
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  return createMockQuery<TData, TError>(undefined, {
    data: undefined,
    error,
    isError: true,
    isLoadingError: true,
    isPending: false,
    isSuccess: false,
    status: 'error',
    errorUpdatedAt: Date.now(),
    errorUpdateCount: 1,
    failureCount: 1,
    failureReason: error,
    ...overrides,
  })
}

/**
 * Creates a mock mutation hook with standard return values
 */
export function createMockMutation<
  TData = unknown,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  overrides?: Partial<UseMutationResult<TData, TError, TVariables, TContext>>,
): UseMutationResult<TData, TError, TVariables, TContext> {
  const baseResult: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {
    data: undefined,
    error: null,
    isError: false,
    isIdle: true,
    isPending: false,
    isSuccess: false,
    status: 'idle',
    variables: undefined,
    context: undefined,
    failureCount: 0,
    failureReason: null,
    isPaused: false,
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    reset: vi.fn(),
    submittedAt: 0,
  }

  return {
    ...baseResult,
    ...overrides,
  } as UseMutationResult<TData, TError, TVariables, TContext>
}

/**
 * Creates a mock mutation hook in pending state
 */
export function createMockPendingMutation<
  TData = unknown,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  overrides?: Partial<UseMutationResult<TData, TError, TVariables, TContext>>,
): UseMutationResult<TData, TError, TVariables, TContext> {
  return createMockMutation<TData, TError, TVariables, TContext>({
    isIdle: false,
    isPending: true,
    status: 'pending',
    ...overrides,
  })
}

/**
 * Creates a mock mutation hook in success state
 */
export function createMockSuccessMutation<
  TData = unknown,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  data: TData,
  overrides?: Partial<UseMutationResult<TData, TError, TVariables, TContext>>,
): UseMutationResult<TData, TError, TVariables, TContext> {
  return createMockMutation<TData, TError, TVariables, TContext>({
    data,
    isIdle: false,
    isSuccess: true,
    status: 'success',
    ...overrides,
  })
}

/**
 * Common mock patterns for API hooks
 */
export const mockApiHooks = {
  mockSuccessQuery: <TData = unknown>(data: TData) => vi.fn(() => createMockQuery(data)),
  mockLoadingQuery: () => vi.fn(() => createMockLoadingQuery()),
  mockErrorQuery: <TError = Error>(error: TError) => vi.fn(() => createMockErrorQuery(error)),
  mockSuccessMutation: <TData = unknown>(data: TData) =>
    vi.fn(() => createMockSuccessMutation(data)),
  mockPendingMutation: () => vi.fn(() => createMockPendingMutation()),
}

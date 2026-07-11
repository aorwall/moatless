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
  data: TData | undefined,
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  const hasData = data !== undefined

  const baseResult = {
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
    promise: Promise.resolve(data as TData),
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
  const base: Partial<UseQueryResult<TData, TError>> = {
    data: undefined,
    error: null,
    isError: false,
    isLoading: true,
    isLoadingError: false,
    isRefetchError: false,
    isSuccess: false,
    isPending: true,
    status: 'pending',
    dataUpdatedAt: 0,
    errorUpdatedAt: 0,
    failureCount: 0,
    failureReason: null,
    errorUpdateCount: 0,
    isFetched: false,
    isFetchedAfterMount: false,
    isFetching: true,
    isInitialLoading: true,
    isPaused: false,
    isPlaceholderData: false,
    isRefetching: false,
    isStale: false,
    refetch: vi.fn().mockResolvedValue({
      data: undefined,
      error: null,
      isError: false,
      isSuccess: false,
    }),
    fetchStatus: 'fetching',
    promise: Promise.resolve(undefined as TData),
  }

  return {
    ...base,
    ...overrides,
  } as UseQueryResult<TData, TError>
}

/**
 * Creates a mock query hook in error state
 */
export function createMockErrorQuery<TData = unknown, TError = Error>(
  error: TError,
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  const base: Partial<UseQueryResult<TData, TError>> = {
    data: undefined,
    error,
    isError: true,
    isLoading: false,
    isLoadingError: true,
    isRefetchError: false,
    isSuccess: false,
    isPending: false,
    status: 'error',
    dataUpdatedAt: 0,
    errorUpdatedAt: Date.now(),
    failureCount: 1,
    failureReason: error,
    errorUpdateCount: 1,
    isFetched: true,
    isFetchedAfterMount: true,
    isFetching: false,
    isInitialLoading: false,
    isPaused: false,
    isPlaceholderData: false,
    isRefetching: false,
    isStale: false,
    refetch: vi.fn().mockResolvedValue({
      data: undefined,
      error,
      isError: true,
      isSuccess: false,
    }),
    fetchStatus: 'idle',
    // Create a rejected promise that's already handled to avoid unhandled rejection warnings
    promise: Promise.reject(error).catch(() => {
      /* noop: rejection is intentional in error-state mock */
    }) as Promise<TData>,
  }

  return {
    ...base,
    ...overrides,
  } as UseQueryResult<TData, TError>
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
  const base: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {
    data: undefined,
    error: null,
    isError: false,
    isIdle: false,
    isPending: true,
    isSuccess: false,
    status: 'pending',
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
  const base: Partial<UseMutationResult<TData, TError, TVariables, TContext>> = {
    data,
    error: null,
    isError: false,
    isIdle: false,
    isPending: false,
    isSuccess: true,
    status: 'success',
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

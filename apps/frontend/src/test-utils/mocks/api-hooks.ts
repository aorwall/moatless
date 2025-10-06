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
export function createMockQuery<TData = any, TError = Error>(
  data: TData | undefined = undefined,
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  const hasData = data !== undefined

  // Use 'as any' to bypass TypeScript's strict type checking for test mocks
  // This is acceptable in test files where we need flexibility
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
    promise: Promise.resolve(data),
  } as any

  return {
    ...baseResult,
    ...overrides,
  } as UseQueryResult<TData, TError>
}

/**
 * Creates a mock query hook in loading state
 */
export function createMockLoadingQuery<TData = any, TError = Error>(
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
  } as any)
}

/**
 * Creates a mock query hook in error state
 */
export function createMockErrorQuery<TData = any, TError = Error>(
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
  } as any)
}

/**
 * Creates a mock mutation hook with standard return values
 */
export function createMockMutation<
  TData = any,
  TError = Error,
  TVariables = void,
  TContext = unknown,
>(
  overrides?: Partial<UseMutationResult<TData, TError, TVariables, TContext>>,
): UseMutationResult<TData, TError, TVariables, TContext> {
  return {
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
    ...overrides,
  } as any
}

/**
 * Creates a mock mutation hook in pending state
 */
export function createMockPendingMutation<
  TData = any,
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
  } as any)
}

/**
 * Creates a mock mutation hook in success state
 */
export function createMockSuccessMutation<
  TData = any,
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
  } as any)
}

/**
 * Common mock patterns for API hooks
 */
export const mockApiHooks = {
  mockSuccessQuery: <TData = any>(data: TData) => vi.fn(() => createMockQuery(data)),
  mockLoadingQuery: () => vi.fn(() => createMockLoadingQuery()),
  mockErrorQuery: <TError = Error>(error: TError) => vi.fn(() => createMockErrorQuery(error)),
  mockSuccessMutation: <TData = any>(data: TData) => vi.fn(() => createMockSuccessMutation(data)),
  mockPendingMutation: () => vi.fn(() => createMockPendingMutation()),
}

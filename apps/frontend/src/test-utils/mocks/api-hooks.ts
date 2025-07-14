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
  return {
    data,
    error: null,
    isError: false,
    isLoading: false,
    isLoadingError: false,
    isRefetchError: false,
    isSuccess: Boolean(data),
    status: data ? 'success' : 'idle',
    dataUpdatedAt: data ? Date.now() : 0,
    errorUpdatedAt: 0,
    failureCount: 0,
    failureReason: null,
    errorUpdateCount: 0,
    isFetched: Boolean(data),
    isFetchedAfterMount: Boolean(data),
    isFetching: false,
    isInitialLoading: false,
    isPaused: false,
    isPlaceholderData: false,
    isPreviousData: false,
    isRefetching: false,
    isStale: false,
    refetch: vi.fn().mockResolvedValue({ data, error: null, isError: false, isSuccess: true }),
    remove: vi.fn(),
    fetchStatus: 'idle',
    ...overrides,
  } as UseQueryResult<TData, TError>
}

/**
 * Creates a mock query hook in loading state
 * @param overrides - Optional overrides
 * @returns Mock query result in loading state
 */
export function createMockLoadingQuery<TData = any, TError = Error>(
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  return createMockQuery<TData, TError>(undefined, {
    isLoading: true,
    isInitialLoading: true,
    isFetching: true,
    status: 'loading',
    fetchStatus: 'fetching',
    ...overrides,
  })
}

/**
 * Creates a mock query hook in error state
 * @param error - The error to return
 * @param overrides - Optional overrides
 * @returns Mock query result in error state
 */
export function createMockErrorQuery<TData = any, TError = Error>(
  error: TError,
  overrides?: Partial<UseQueryResult<TData, TError>>,
): UseQueryResult<TData, TError> {
  return createMockQuery<TData, TError>(undefined, {
    error,
    isError: true,
    isLoadingError: true,
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
 * @param overrides - Optional overrides for the mutation result
 * @returns Mock mutation result
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
    ...overrides,
  } as unknown as UseMutationResult<TData, TError, TVariables, TContext>
}

/**
 * Creates a mock mutation hook in pending state
 * @param overrides - Optional overrides
 * @returns Mock mutation result in pending state
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
  })
}

/**
 * Creates a mock mutation hook in success state
 * @param data - The data to return
 * @param overrides - Optional overrides
 * @returns Mock mutation result in success state
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
  })
}

/**
 * Common mock patterns for API hooks
 */
export const mockApiHooks = {
  /**
   * Mock successful data fetch
   */
  mockSuccessQuery: <TData = any>(data: TData) => vi.fn(() => createMockQuery(data)),

  /**
   * Mock loading state
   */
  mockLoadingQuery: () => vi.fn(() => createMockLoadingQuery()),

  /**
   * Mock error state
   */
  mockErrorQuery: <TError = Error>(error: TError) => vi.fn(() => createMockErrorQuery(error)),

  /**
   * Mock successful mutation
   */
  mockSuccessMutation: <TData = any>(data: TData) => vi.fn(() => createMockSuccessMutation(data)),

  /**
   * Mock pending mutation
   */
  mockPendingMutation: () => vi.fn(() => createMockPendingMutation()),
}

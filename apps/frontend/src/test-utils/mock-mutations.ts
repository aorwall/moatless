import { vi } from 'vitest'

/**
 * Creates a mock mutation result with all required properties from React Query's UseMutationResult
 * Since test files allow `any` types, we use `any` to avoid complex type gymnastics
 */
export function createMockMutationResult(overrides: any = {}): any {
  const base = {
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

  return { ...base, ...overrides }
}

/**
 * Creates a mock mutation in a pending state
 */
export function createPendingMutation(overrides: any = {}): any {
  return createMockMutationResult({
    isIdle: false,
    isPending: true,
    status: 'pending',
    ...overrides,
  })
}

/**
 * Creates a mock mutation in a success state
 */
export function createSuccessMutation<TData = unknown>(data: TData, overrides: any = {}): any {
  return createMockMutationResult({
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
export function createErrorMutation<TError = Error>(error: TError, overrides: any = {}): any {
  return createMockMutationResult({
    error,
    isIdle: false,
    isError: true,
    status: 'error',
    failureCount: 1,
    failureReason: error,
    ...overrides,
  })
}

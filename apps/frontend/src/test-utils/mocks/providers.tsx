import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type React from 'react'

/**
 * Mock provider wrappers for consistent test setup
 */

/**
 * Creates a wrapper with React Query provider for testing
 * @param options - Optional QueryClient configuration
 * @returns Wrapper component with React Query provider
 */
export function createQueryWrapper(options?: {
  defaultOptions?: Parameters<typeof QueryClient>[0]['defaultOptions']
}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        ...options?.defaultOptions?.queries,
      },
      mutations: {
        retry: false,
        ...options?.defaultOptions?.mutations,
      },
    },
  })

  return function QueryWrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  }
}

/**
 * Creates a wrapper with all common providers for testing
 * @param additionalProviders - Additional providers to wrap around children
 * @returns Wrapper component with all providers
 */
export function createAllProvidersWrapper(
  additionalProviders?: Array<React.ComponentType<{ children: React.ReactNode }>>,
) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return function AllProvidersWrapper({ children }: { children: React.ReactNode }) {
    let wrappedChildren = children

    // Wrap with additional providers if provided
    if (additionalProviders) {
      for (const Provider of additionalProviders.reverse()) {
        wrappedChildren = <Provider>{wrappedChildren}</Provider>
      }
    }

    // Wrap with common providers
    return <QueryClientProvider client={queryClient}>{wrappedChildren}</QueryClientProvider>
  }
}

/**
 * Default test wrapper with React Query
 */
export const defaultWrapper = createQueryWrapper()

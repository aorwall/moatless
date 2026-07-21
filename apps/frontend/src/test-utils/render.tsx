import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { RenderOptions } from '@testing-library/react'
import { render } from '@testing-library/react'
import type { ReactElement, ReactNode } from 'react'

interface AllTheProvidersProps {
  children: ReactNode
}

export function createTestQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        gcTime: 0,
        staleTime: 0,
      },
      mutations: {
        retry: false,
      },
    },
  })
}

// Mock providers to prevent test failures
const MockSidebarProvider = ({ children }: { children: ReactNode }) => children
const MockTaskUIProvider = ({ children }: { children: ReactNode }) => children
const MockTooltipProvider = ({ children }: { children: ReactNode }) => children

function AllTheProviders({ children }: AllTheProvidersProps) {
  const queryClient = createTestQueryClient()

  return (
    <QueryClientProvider client={queryClient}>
      <MockTooltipProvider>
        <MockSidebarProvider>
          <MockTaskUIProvider>{children}</MockTaskUIProvider>
        </MockSidebarProvider>
      </MockTooltipProvider>
    </QueryClientProvider>
  )
}

const customRender = (ui: ReactElement, options?: Omit<RenderOptions, 'wrapper'>) =>
  render(ui, { wrapper: AllTheProviders, ...options })

// Re-export everything
export * from '@testing-library/react'
export { customRender as render }

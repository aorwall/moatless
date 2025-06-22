import type { ReactElement, ReactNode } from 'react';
import type { RenderOptions } from '@testing-library/react';
import { render } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
// Mock router types
interface MockRouter {
  subscribe: () => () => void;
  state: { location: { pathname: string } };
}

const createMockRouter = (route: string): MockRouter => ({
  subscribe: () => () => {},
  state: { location: { pathname: route } },
});

interface AllTheProvidersProps {
  children: ReactNode;
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
    logger: {
      log: () => {},
      warn: () => {},
      error: () => {},
    },
  });
}

function AllTheProviders({ children }: AllTheProvidersProps) {
  const queryClient = createTestQueryClient();

  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

const customRender = (
  ui: ReactElement,
  options?: Omit<RenderOptions, 'wrapper'>,
) => render(ui, { wrapper: AllTheProviders, ...options });

// Re-export everything
export * from '@testing-library/react';
export { customRender as render };

// Helper to render with router context
export function renderWithRouter(
  ui: ReactElement,
  {
    route = '/',
    ...renderOptions
  }: {
    route?: string;
  } & Omit<RenderOptions, 'wrapper'> = {},
) {
  const queryClient = createTestQueryClient();

  // For testing, we just render with query client
  // Router mocking is handled in individual test files
  return render(
    <QueryClientProvider client={queryClient}>
      {ui}
    </QueryClientProvider>,
    renderOptions,
  );
}

// Helper to create wrapper for renderHook
export function createWrapper() {
  const queryClient = createTestQueryClient();

  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}
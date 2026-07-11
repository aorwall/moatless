import { QueryClientProvider } from '@tanstack/react-query'
import { createRouter, RouterProvider } from '@tanstack/react-router'
import { StrictMode } from 'react'
import ReactDOM from 'react-dom/client'
import { Toaster } from 'sonner'

// Import the generated route tree
import { routeTree } from './routeTree.gen'

import '@/lib/styles/globals.css'

import { queryClient } from '@/lib/api/query-client'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { FeatureFlagsProvider } from '@/lib/contexts/feature-flags-context'
import { Page404 } from '@/lib/pages/404'

// Last-resort logging for promise rejections that escape all local handlers
// (fire-and-forget mutations, event handlers). Error boundaries can't catch
// these, so without this listener they vanish silently.
window.addEventListener('unhandledrejection', (event) => {
  // biome-ignore lint/suspicious/noConsole: last-resort sink for floating rejections
  console.error('Unhandled promise rejection:', event.reason)
})

// Create a new router instance
const router = createRouter({
  routeTree,
  context: {},
  defaultPreload: 'intent',
  scrollRestoration: true,
  defaultStructuralSharing: true,
  defaultPreloadStaleTime: 0,
  defaultPendingComponent: () => (
    <div className="mx-auto">
      <p>Loading...</p>
    </div>
  ),
  defaultNotFoundComponent: () => <Page404 />,
})

// Register the router instance for type safety
declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}

// Render the app
const rootElement = document.getElementById('app')
if (rootElement && !rootElement.innerHTML) {
  const root = ReactDOM.createRoot(rootElement)
  root.render(
    <StrictMode>
      <FeatureFlagsProvider>
        <QueryClientProvider client={queryClient}>
          <ErrorBoundary componentName="Application">
            <RouterProvider router={router} />
          </ErrorBoundary>
          <Toaster position="bottom-right" />
        </QueryClientProvider>
      </FeatureFlagsProvider>
    </StrictMode>,
  )
}
//           <ReactQueryDevtools initialIsOpen={false} buttonPosition={'bottom-left'} />
// If you want to start measuring performance in your app, pass a function
// to log results (for example: reportWebVitals(console.log))
// or send to an analytics endpoint. Learn more: https://bit.ly/CRA-vitals
// reportWebVitals()

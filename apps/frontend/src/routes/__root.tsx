import { createRootRoute, HeadContent, Outlet, useRouterState } from '@tanstack/react-router'
import { useEffect, useRef } from 'react'

import { AuthGate } from '@/features/auth/components/auth-gate'
import { GlobalEventStreamProvider } from '@/features/events'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ServerDownGate } from '@/lib/components/server-down-gate'
import { Layout } from '@/lib/layout'

const title = 'Moatless Vibe'
const description = 'Moatless Vibe'
const url = 'https://moatless.ai'
const ogImgUrl =
  'https://og.sznm.dev/api/generate?heading=Moatless+Vibe&text=Moatless+Vibe&template=color'

export const Route = createRootRoute({
  head: () => ({
    meta: [
      {
        title,
      },
      {
        name: 'description',
        content: description,
      },
      // viewport is set in index.html (with iOS-specific maximum-scale to
      // prevent auto-zoom on input focus) — do not redeclare it here
      {
        name: 'application-name',
        content: title,
      },
      {
        name: 'apple-mobile-web-app-capable',
        content: 'yes',
      },
      {
        name: 'apple-mobile-web-app-status-bar-style',
        content: 'default',
      },
      {
        name: 'apple-mobile-web-app-title',
        content: title,
      },
      {
        name: 'format-detection',
        content: 'telephone=no',
      },
      {
        name: 'mobile-web-app-capable',
        content: 'yes',
      },
      {
        name: 'theme-color',
        content: '#000000',
      },
      {
        name: 'og:type',
        content: 'website',
      },
      {
        name: 'og:url',
        content: url,
      },
      {
        name: 'og:title',
        content: title,
      },
      {
        name: 'og:description',
        content: description,
      },
      {
        name: 'og:image',
        content: ogImgUrl,
      },
      {
        name: 'twitter:card',
        content: 'summary_large_image',
      },
      {
        name: 'twitter:url',
        content: url,
      },
      {
        name: 'twitter:title',
        content: title,
      },
      {
        name: 'twitter:description',
        content: description,
      },
      {
        name: 'twitter:image',
        content: ogImgUrl,
      },
    ],
    links: [
      {
        rel: 'icon',
        href: '/favicon.ico',
      },
      {
        rel: 'apple-touch-icon',
        href: '/apple-touch-icon-180x180.png',
      },
    ],
  }),
  component: RootComponent,
})

function RootComponent() {
  const { pathname } = useRouterState({ select: (s) => s.location })
  const isPublicRoute = pathname === '/login' || pathname.startsWith('/auth/')

  // Move focus to the main content region after client-side navigation so
  // screen reader and keyboard users get a clear "new page" signal (SPA route
  // changes don't reset focus the way full page loads do).
  const previousPathname = useRef(pathname)
  useEffect(() => {
    if (previousPathname.current === pathname) {
      return
    }
    previousPathname.current = pathname
    document.getElementById('main-content')?.focus()
  }, [pathname])

  if (isPublicRoute) {
    return (
      <ServerDownGate>
        <HeadContent />
        <ErrorBoundary componentName="Page">
          <Outlet />
        </ErrorBoundary>
      </ServerDownGate>
    )
  }

  return (
    <ServerDownGate>
      <HeadContent />
      <ErrorBoundary>
        <AuthGate>
          <GlobalEventStreamProvider>
            <Layout>
              <Outlet />
            </Layout>
          </GlobalEventStreamProvider>
        </AuthGate>
      </ErrorBoundary>
    </ServerDownGate>
  )
}

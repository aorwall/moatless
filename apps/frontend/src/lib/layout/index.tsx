import type { ReactNode } from 'react'

import { ComposeDock } from '@/features/tasks/components/compose/compose-dock'
import { ComposeTaskProvider } from '@/features/tasks/contexts/compose-task-provider'
import { ThemeProvider } from '@/lib/components/theme-provider'

type LayoutProps = {
  children: ReactNode
}

export const Layout = ({ children }: LayoutProps) => {
  return (
    <ThemeProvider>
      <ComposeTaskProvider>
        <a
          href="#main-content"
          className="sr-only focus:not-sr-only focus:absolute focus:left-2 focus:top-2 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:shadow-md"
        >
          Skip to main content
        </a>
        <div className="flex h-full w-full bg-background text-foreground">
          <main
            id="main-content"
            tabIndex={-1}
            className="flex-1 overflow-hidden h-full outline-none"
          >
            {children}
          </main>
        </div>
        <ComposeDock />
      </ComposeTaskProvider>
    </ThemeProvider>
  )
}

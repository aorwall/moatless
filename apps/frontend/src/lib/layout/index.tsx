import type { ReactNode } from 'react'

import { SettingsDialogProvider } from '@/features/settings/contexts/settings-dialog-provider'
import { SetupRequirementsBanner } from '@/features/setup'
import { ComposeDock } from '@/features/tasks/components/compose/compose-dock'
import { ComposeTaskProvider } from '@/features/tasks/contexts/compose-task-provider'
import { ThemeProvider } from '@/lib/components/theme-provider'

type LayoutProps = {
  children: ReactNode
}

export const Layout = ({ children }: LayoutProps) => {
  return (
    <ThemeProvider>
      <SettingsDialogProvider>
        <ComposeTaskProvider>
          <a
            href="#main-content"
            className="sr-only focus:not-sr-only focus:absolute focus:left-2 focus:top-2 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:shadow-md"
          >
            Skip to main content
          </a>
          <div className="flex h-full w-full flex-col bg-background text-foreground">
            <SetupRequirementsBanner />
            {/* `min-h-0` or the `overflow-hidden` panes inside task pages
                overflow the row instead of scrolling. */}
            <div className="flex min-h-0 w-full flex-1">
              <main
                id="main-content"
                tabIndex={-1}
                className="flex-1 overflow-hidden h-full outline-none"
              >
                {children}
              </main>
            </div>
          </div>
          <ComposeDock />
        </ComposeTaskProvider>
      </SettingsDialogProvider>
    </ThemeProvider>
  )
}

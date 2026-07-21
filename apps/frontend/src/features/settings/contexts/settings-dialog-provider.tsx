import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from 'react'

import { SettingsDialog } from '../components/settings-dialog'

interface SettingsDialogContextValue {
  openSettings: (section?: string) => void
  closeSettings: () => void
}

const SettingsDialogContext = createContext<SettingsDialogContextValue | null>(null)

/**
 * App-level mount of the settings dialog, so anything under the layout (the
 * setup banner, for one) can deep-link into a section without owning dialog
 * state of its own.
 *
 * Route-local mounts of `SettingsDialog` still exist and are unaffected — only
 * one dialog is open at a time.
 */
export function SettingsDialogProvider({ children }: { children: ReactNode }) {
  const [section, setSection] = useState<string | null>(null)

  const openSettings = useCallback((next = 'profile') => setSection(next), [])
  const closeSettings = useCallback(() => setSection(null), [])

  const value = useMemo(() => ({ openSettings, closeSettings }), [openSettings, closeSettings])

  return (
    <SettingsDialogContext.Provider value={value}>
      {children}
      {section !== null && (
        // `SettingsDialog` seeds `activeSection` from `defaultSection` in a
        // `useState` initializer, so remount on change or a second open would
        // land on the previous section.
        <SettingsDialog
          key={section}
          open
          onOpenChange={(open) => {
            if (!open) {
              closeSettings()
            }
          }}
          defaultSection={section}
        />
      )}
    </SettingsDialogContext.Provider>
  )
}

export function useSettingsDialog(): SettingsDialogContextValue {
  const context = useContext(SettingsDialogContext)
  if (!context) {
    throw new Error('useSettingsDialog must be used within a SettingsDialogProvider')
  }
  return context
}

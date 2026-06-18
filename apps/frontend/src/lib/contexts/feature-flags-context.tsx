import type { FeatureFlagsResponse } from '@moatless/api-client/sandbox/model'
import { createContext, type ReactNode, useContext, useEffect, useState } from 'react'
import { apiClient } from '@/lib/api/client'
import { isServerUnavailable, serverStatusStore } from '@/lib/api/server-status'
import type { FeatureFlagKey } from '@/lib/types/feature-flags'

interface SlackConfig {
  teamId?: string
}

interface FeatureFlagsContextValue {
  flags: FeatureFlagsResponse | null
  isLoading: boolean
  isEnabled: (flag: FeatureFlagKey) => boolean
  getSlackConfig: () => SlackConfig | undefined
}

const FeatureFlagsContext = createContext<FeatureFlagsContextValue | undefined>(undefined)

interface FeatureFlagsProviderProps {
  children: ReactNode
}

/**
 * Provider for feature flags
 * Fetches flags from the API once on mount and provides them via context
 */
export function FeatureFlagsProvider({ children }: FeatureFlagsProviderProps) {
  const [flags, setFlags] = useState<FeatureFlagsResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)

  useEffect(() => {
    const fetchFlags = async () => {
      try {
        const response = await apiClient.get<FeatureFlagsResponse>('/api/v1/feature-flags')
        setFlags(response)
      } catch (err) {
        // This bootstrap call bypasses TanStack Query, so flag the global
        // server-status store directly when the backend is unreachable.
        if (isServerUnavailable(err)) {
          serverStatusStore.setUnavailable(true)
        }
        // Set default flags object on error so the app continues to work
        setFlags({
          netlify_publish: false,
          figma_clipboard_paste: false,
          gitlab_enabled: false,
          show_claude_settings: false,
          show_codex_settings: true,
          show_model_selector: true,
        })
      } finally {
        setIsLoading(false)
      }
    }

    fetchFlags()
  }, [])

  const isEnabled = (flag: FeatureFlagKey): boolean => {
    if (!flags) {
      return false
    }
    const flagKey = flag.toLowerCase() as keyof FeatureFlagsResponse
    const value = flags[flagKey]
    return typeof value === 'boolean' ? value : false
  }

  const getSlackConfig = (): SlackConfig | undefined => {
    return flags?.slack ?? undefined
  }

  return (
    <FeatureFlagsContext.Provider value={{ flags, isLoading, isEnabled, getSlackConfig }}>
      {children}
    </FeatureFlagsContext.Provider>
  )
}

/**
 * Hook to access feature flags context
 * @throws Error if used outside of FeatureFlagsProvider
 */
export function useFeatureFlagsContext() {
  const context = useContext(FeatureFlagsContext)
  if (!context) {
    throw new Error('useFeatureFlagsContext must be used within a FeatureFlagsProvider')
  }
  return context
}

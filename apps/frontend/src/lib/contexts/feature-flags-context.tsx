import type { FeatureFlagsResponse } from '@moatless/api-client/sandbox/model'
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react'
import { apiClient } from '@/lib/api/client'
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
      } catch {
        // Feature flags are non-critical: on any failure (including a 503 while
        // the backend restarts) fall back to safe defaults so the app keeps
        // working. Only the auth-session probe drives the full-page
        // `ServerDownGate`, so this must NOT flag the global server status.
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

    fetchFlags().catch(() => {
      /* ignore: fetchFlags handles all errors internally via try/catch/finally */
    })
  }, [])

  const isEnabled = useCallback(
    (flag: FeatureFlagKey): boolean => {
      if (!flags) {
        return false
      }
      const flagKey = flag.toLowerCase() as keyof FeatureFlagsResponse
      const value = flags[flagKey]
      return typeof value === 'boolean' ? value : false
    },
    [flags],
  )

  const getSlackConfig = useCallback((): SlackConfig | undefined => {
    return flags?.slack ?? undefined
  }, [flags])

  const value = useMemo(
    () => ({ flags, isLoading, isEnabled, getSlackConfig }),
    [flags, isLoading, isEnabled, getSlackConfig],
  )

  return <FeatureFlagsContext value={value}>{children}</FeatureFlagsContext>
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

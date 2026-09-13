import type { FeatureFlagsResponse } from '@moatless/api-client/sandbox/model'
import { createContext, type ReactNode, useContext, useEffect, useState } from 'react'
import { apiClient } from '@/lib/api/client'
import type { FeatureFlagKey } from '@/lib/types/feature-flags'

interface FeatureFlagsContextValue {
  flags: FeatureFlagsResponse | null
  isLoading: boolean
  isEnabled: (flag: FeatureFlagKey) => boolean
}

const FeatureFlagsContext = createContext<FeatureFlagsContextValue | undefined>(undefined)

const FEATURE_FLAGS_FALLBACK: FeatureFlagsResponse = {
  netlify_publish: false,
  gitlab_enabled: false,
  forgejo_enabled: false,
  show_claude_settings: false,
  show_codex_settings: true,
  show_model_selector: true,
  device_pairing: false,
  workspace_settings: false,
}

async function loadFeatureFlags(): Promise<FeatureFlagsResponse> {
  try {
    return await apiClient.get<FeatureFlagsResponse>('/api/v1/feature-flags')
  } catch {
    // Feature flags are non-critical: on any failure (including a 503 while
    // the backend restarts) fall back to safe defaults so the app keeps
    // working. Only the auth-session probe drives the full-page
    // `ServerDownGate`, so this must NOT flag the global server status.
    return FEATURE_FLAGS_FALLBACK
  }
}

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
    let cancelled = false

    loadFeatureFlags()
      .then((response) => {
        if (!cancelled) {
          setFlags(response)
          setIsLoading(false)
        }
      })
      .catch(() => {
        // loadFeatureFlags already converts failures to the safe fallback.
      })

    return () => {
      cancelled = true
    }
  }, [])

  const isEnabled = (flag: FeatureFlagKey): boolean => {
    if (!flags) {
      return false
    }
    const flagKey = flag.toLowerCase() as keyof FeatureFlagsResponse
    const value = flags[flagKey]
    return typeof value === 'boolean' ? value : false
  }

  const value = {
    flags,
    isLoading,
    isEnabled,
  }

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

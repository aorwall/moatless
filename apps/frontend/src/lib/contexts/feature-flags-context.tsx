import type { FeatureFlagKey, FeatureFlagsResponse } from '@moatless/types'
import { createContext, type ReactNode, useContext, useEffect, useState } from 'react'
import { apiClient } from '@/lib/api/client'

interface FeatureFlagsContextValue {
  flags: FeatureFlagsResponse | null
  isLoading: boolean
  isEnabled: (flag: FeatureFlagKey) => boolean
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
        // Set empty flags object on error so the app continues to work
        setFlags({
          netlify_publish: false,
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
    return flags[flagKey] ?? false
  }

  return (
    <FeatureFlagsContext.Provider value={{ flags, isLoading, isEnabled }}>
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

import { useFeatureFlagsContext } from '@/lib/contexts/feature-flags-context'
import type { FeatureFlagKey } from '@/lib/types/feature-flags'

/**
 * Hook to check if a specific feature flag is enabled
 *
 * @example
 * ```tsx
 * function MyComponent() {
 *   const isEnabled = useFeatureFlag('NETLIFY_PUBLISH')
 *
 *   if (!isEnabled) return null
 *
 *   return <PublishButton />
 * }
 * ```
 */
export function useFeatureFlag(flag: FeatureFlagKey): boolean {
  const { isEnabled } = useFeatureFlagsContext()
  return isEnabled(flag)
}

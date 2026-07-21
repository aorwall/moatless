import type { BaseDiffOptions } from '@pierre/diffs/react'
import { useTheme } from '@/lib/components/theme-provider'

const PIERRE_THEMES = { dark: 'github-dark', light: 'github-light' } as const

export function usePierreOptions(): Pick<BaseDiffOptions, 'theme' | 'themeType'> {
  const { theme } = useTheme()
  return {
    theme: PIERRE_THEMES,
    themeType: theme,
  }
}

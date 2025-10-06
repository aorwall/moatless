import { useEffect, useState } from 'react'
import type { ConfigurationDialogProps } from './configuration-dialog-desktop'
import ConfigurationDialogDesktop from './configuration-dialog-desktop'
import ConfigurationDialogMobile from './configuration-dialog-mobile'

// Custom hook to detect screen size
function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false)

  useEffect(() => {
    const media = window.matchMedia(query)

    // Set initial value
    setMatches(media.matches)

    // Create event listener
    const listener = (event: MediaQueryListEvent) => {
      setMatches(event.matches)
    }

    // Add event listener
    media.addEventListener('change', listener)

    // Clean up
    return () => {
      media.removeEventListener('change', listener)
    }
  }, [query])

  return matches
}

// Responsive wrapper that conditionally renders desktop or mobile version
export default function ResponsiveConfigurationDialog(props: ConfigurationDialogProps) {
  // Check if screen is at least 640px wide (sm breakpoint)
  const isDesktop = useMediaQuery('(min-width: 640px)')

  // Only render one dialog based on screen size
  if (isDesktop) {
    return <ConfigurationDialogDesktop {...props} />
  }

  return <ConfigurationDialogMobile {...props} />
}

// Re-export types for convenience
export type {
  ConfigurationDialogProps,
  ConfigurationSection,
} from './configuration-dialog-desktop'

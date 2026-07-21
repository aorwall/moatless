import { useNavigate, useSearch } from '@tanstack/react-router'

/**
 * Preview device mode. Controls the simulated device width the preview iframe is
 * rendered at.
 */
export type DeviceMode = 'mobile' | 'tablet' | 'desktop'

/**
 * Upper bound for the `?path=` preview param. A preview route (plus any query
 * of the previewed app) is never legitimately longer than this. The cap breaks
 * a self-preview feedback loop: when the preview target is this app itself, the
 * iframe reports its own `pathname + search` — which already contains `?path=` —
 * and storing it verbatim would nest the URL one encoding layer deeper on every
 * navigation, growing unboundedly.
 */
const MAX_PREVIEW_PATH_LENGTH = 2048

/** Loose view of the route search params the preview hooks touch. */
interface PreviewSearch {
  path?: string
  device?: DeviceMode
}

/**
 * Read the route search params loosely, independent of the task route's search
 * schema (so `features/preview` doesn't depend on `features/tasks`).
 */
function usePreviewSearch(): PreviewSearch {
  return useSearch({ strict: false }) as PreviewSearch
}

interface UsePreviewPathReturn {
  previewPath: string
  setPreviewPath: (path: string) => void
}

/**
 * Hook for managing preview path state via the `?path=` URL search param.
 *
 * The preview path is stored in the URL, making it:
 * - Shareable via URL
 * - Persistent across page refreshes
 * - Compatible with browser back/forward navigation
 */
export function usePreviewPath(): UsePreviewPathReturn {
  const navigate = useNavigate()
  const search = usePreviewSearch()

  const previewPath = search.path ?? '/'

  const setPreviewPath = (path: string) => {
    // Drop pathological values (see MAX_PREVIEW_PATH_LENGTH) instead of writing
    // them to the URL; storing them would feed the self-preview recursion.
    if (path.length > MAX_PREVIEW_PATH_LENGTH) {
      return
    }

    navigate({
      to: '.',
      search: (prev) => ({ ...(prev as PreviewSearch), path }),
      replace: true, // Use replace to avoid polluting browser history
    }).catch(() => {
      /* ignore: navigate() for URL param updates only rejects on framework errors */
    })
  }

  return { previewPath, setPreviewPath }
}

interface UsePreviewDeviceReturn {
  device: DeviceMode
  setDevice: (mode: DeviceMode) => void
}

/**
 * Hook for managing the preview device mode via the `?device=` URL search param.
 *
 * The device mode is stored in the URL, making it:
 * - Shareable via URL (useful for bug reports at specific device sizes)
 * - Persistent across page refreshes
 * - Compatible with browser back/forward navigation
 */
export function usePreviewDevice(): UsePreviewDeviceReturn {
  const navigate = useNavigate()
  const search = usePreviewSearch()

  const device = search.device ?? 'desktop'

  const setDevice = (mode: DeviceMode) => {
    navigate({
      to: '.',
      search: (prev) => ({ ...(prev as PreviewSearch), device: mode }),
      replace: true, // Use replace to avoid polluting browser history
    }).catch(() => {
      /* ignore: navigate() for URL param updates only rejects on framework errors */
    })
  }

  return { device, setDevice }
}

import { useNavigate, useSearch } from '@tanstack/react-router'

interface UseSelectedFileReturn {
  selectedFile: string | undefined
  setSelectedFile: (file: string | undefined) => void
}

/** Loose view of the task route search params this hook touches. */
interface FileSearch {
  file?: string
}

/**
 * Read the route search params loosely, independent of the task route's search
 * schema (so `features/files` doesn't depend on `features/tasks`).
 */
function useFileSearchParams(): FileSearch {
  return useSearch({ strict: false }) as FileSearch
}

/**
 * Manage the selected file path via the `?file=` URL search param, making the
 * selection shareable, refresh-persistent, and back/forward compatible.
 *
 * Lives in `features/files` (not `features/tasks`) so the file explorer stays
 * decoupled from the tasks feature.
 */
export function useSelectedFile(): UseSelectedFileReturn {
  const navigate = useNavigate()
  const search = useFileSearchParams()

  const selectedFile = search.file

  const setSelectedFile = (file: string | undefined) => {
    navigate({
      to: '.',
      search: (prev) => {
        const prevSearch = prev as FileSearch
        // Remove file param if undefined, otherwise set it
        if (file === undefined) {
          const { file: _file, ...rest } = prevSearch
          return rest
        }
        return { ...prevSearch, file }
      },
      replace: true, // Use replace to avoid polluting browser history
    }).catch(() => {
      /* ignore: navigate() for URL param updates only rejects on framework errors */
    })
  }

  return { selectedFile, setSelectedFile }
}

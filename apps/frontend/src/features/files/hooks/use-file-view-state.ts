import { useNavigate, useSearch } from '@tanstack/react-router'
import { useCallback } from 'react'
import type { FileViewMode } from '../components/file-viewer'

interface UseFileViewStateReturn {
  fileView: FileViewMode
  setFileView: (fileView: FileViewMode) => void
  isExplorerCollapsed: boolean
  setExplorerCollapsed: (isCollapsed: boolean) => void
}

/** Loose view of the task route search params this hook touches. */
interface FileViewSearch {
  fileView?: FileViewMode
  filesCollapsed?: boolean
}

function isFileViewMode(value: unknown): value is FileViewMode {
  return value === 'code' || value === 'diff' || value === 'preview'
}

function useFileViewSearchParams(): FileViewSearch {
  return useSearch({ strict: false }) as FileViewSearch
}

/**
 * Manage file viewer UI state via URL search params. This keeps links to a
 * selected file, preview/code/diff mode, and explorer collapsed state stable
 * across refreshes and shareable between users.
 */
export function useFileViewState(): UseFileViewStateReturn {
  const navigate = useNavigate()
  const search = useFileViewSearchParams()

  const fileView = isFileViewMode(search.fileView) ? search.fileView : 'code'
  const isExplorerCollapsed = search.filesCollapsed === true

  const setFileView = useCallback(
    (nextFileView: FileViewMode) => {
      navigate({
        to: '.',
        search: (prev) => {
          const prevSearch = prev as FileViewSearch
          if (nextFileView === 'code') {
            const { fileView: _fileView, ...rest } = prevSearch
            return rest
          }
          return { ...prevSearch, fileView: nextFileView }
        },
        replace: true,
      }).catch(() => {
        /* ignore: URL state update */
      })
    },
    [navigate],
  )

  const setExplorerCollapsed = useCallback(
    (isCollapsed: boolean) => {
      navigate({
        to: '.',
        search: (prev) => {
          const prevSearch = prev as FileViewSearch
          if (!isCollapsed) {
            const { filesCollapsed: _filesCollapsed, ...rest } = prevSearch
            return rest
          }
          return { ...prevSearch, filesCollapsed: true }
        },
        replace: true,
      }).catch(() => {
        /* ignore: URL state update */
      })
    },
    [navigate],
  )

  return { fileView, setFileView, isExplorerCollapsed, setExplorerCollapsed }
}

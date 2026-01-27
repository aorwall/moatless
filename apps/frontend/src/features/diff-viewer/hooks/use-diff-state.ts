import { useCallback, useState } from 'react'

interface DiffState {
  /** Set of collapsed file paths */
  collapsedFiles: Set<string>
  /** Currently selected file in sidebar */
  selectedFile: string | undefined
  /** Search query for filtering files */
  searchQuery: string
}

interface UseDiffStateReturn {
  state: DiffState
  toggleFileCollapsed: (filename: string) => void
  setSelectedFile: (filename: string | undefined) => void
  setSearchQuery: (query: string) => void
  isFileCollapsed: (filename: string) => boolean
  collapseAll: () => void
  expandAll: () => void
}

/**
 * Hook for managing diff viewer UI state
 */
export function useDiffState(filenames: string[] = []): UseDiffStateReturn {
  const [collapsedFiles, setCollapsedFiles] = useState<Set<string>>(new Set())
  const [selectedFile, setSelectedFile] = useState<string | undefined>(undefined)
  const [searchQuery, setSearchQuery] = useState('')

  const toggleFileCollapsed = useCallback((filename: string) => {
    setCollapsedFiles((prev) => {
      const next = new Set(prev)
      if (next.has(filename)) {
        next.delete(filename)
      } else {
        next.add(filename)
      }
      return next
    })
  }, [])

  const isFileCollapsed = useCallback(
    (filename: string) => collapsedFiles.has(filename),
    [collapsedFiles],
  )

  const collapseAll = useCallback(() => {
    setCollapsedFiles(new Set(filenames))
  }, [filenames])

  const expandAll = useCallback(() => {
    setCollapsedFiles(new Set())
  }, [])

  return {
    state: {
      collapsedFiles,
      selectedFile,
      searchQuery,
    },
    toggleFileCollapsed,
    setSelectedFile,
    setSearchQuery,
    isFileCollapsed,
    collapseAll,
    expandAll,
  }
}

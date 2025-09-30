import type { FileSearchResult } from '@moatless/types'
import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useFileAutocomplete } from '../use-file-autocomplete'

describe('useFileAutocomplete - Tier 1 Tests', () => {
  // Mock file search results
  const mockResults: FileSearchResult[] = [
    {
      path: 'src/components/button.tsx',
      absolutePath: '/project/src/components/button.tsx',
      score: 0.95,
    },
    { path: 'src/hooks/use-tasks.ts', absolutePath: '/project/src/hooks/use-tasks.ts', score: 0.9 },
    { path: 'src/utils/helpers.ts', absolutePath: '/project/src/utils/helpers.ts', score: 0.85 },
  ]

  const defaultProps = {
    value: '',
    onValueChange: vi.fn(),
    onSelect: vi.fn(),
    enabled: true,
    onSearch: vi.fn(),
    results: [],
    isLoading: false,
    error: null,
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('@ Detection Logic (handleInputChange)', () => {
    it('should open autocomplete when @ is typed', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      act(() => {
        result.current.handleInputChange('@but', 4)
      })

      expect(result.current.isOpen).toBe(true)
      expect(result.current.searchTerm).toBe('but')
    })

    it('should close autocomplete when @ is removed', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      // Open autocomplete
      act(() => {
        result.current.handleInputChange('@but', 4)
      })
      expect(result.current.isOpen).toBe(true)

      // Remove @
      act(() => {
        result.current.handleInputChange('but', 3)
      })

      expect(result.current.isOpen).toBe(false)
      expect(result.current.searchTerm).toBe('')
    })

    it('should close autocomplete when space is typed after @', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      act(() => {
        result.current.handleInputChange('hello @file name', 12)
      })

      expect(result.current.isOpen).toBe(false)
    })

    it('should extract search term correctly', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      act(() => {
        result.current.handleInputChange('Check @button component', 13)
      })

      expect(result.current.searchTerm).toBe('button')
      expect(result.current.isOpen).toBe(true)
    })

    it('should track cursor position', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      const testValue = 'Look at @src/button'

      act(() => {
        result.current.handleInputChange(testValue, testValue.length)
      })

      // Internal state should have cursor position stored (not exposed in return)
      // We verify indirectly by checking autocomplete opened correctly
      expect(result.current.isOpen).toBe(true)
      expect(result.current.searchTerm).toBe('src/button')
    })

    it('should reset selectedIndex when search term changes', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      // First search
      act(() => {
        result.current.handleInputChange('@but', 4)
      })

      // Second search with different term
      act(() => {
        result.current.handleInputChange('@hook', 5)
      })

      // selectedIndex should reset to 0 on new search
      expect(result.current.selectedIndex).toBe(0)
    })

    it('should not open autocomplete when enabled is false', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          enabled: false,
        }),
      )

      act(() => {
        result.current.handleInputChange('@button', 7)
      })

      expect(result.current.isOpen).toBe(false)
    })

    it('should handle @ at beginning of text', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      act(() => {
        result.current.handleInputChange('@component', 10)
      })

      expect(result.current.isOpen).toBe(true)
      expect(result.current.searchTerm).toBe('component')
    })

    it('should handle multiple @ symbols and use the last one', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      act(() => {
        result.current.handleInputChange('Check @file1 and @file2', 23)
      })

      expect(result.current.isOpen).toBe(true)
      expect(result.current.searchTerm).toBe('file2')
    })
  })

  describe('Debounced Search', () => {
    it('should call onSearch after 300ms delay', async () => {
      const onSearch = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          onSearch,
        }),
      )

      await act(async () => {
        result.current.handleInputChange('@button', 7)
      })

      // Should not call immediately
      expect(onSearch).not.toHaveBeenCalled()

      // Wait for debounce
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 350))
      })

      expect(onSearch).toHaveBeenCalledWith('button')
      expect(onSearch).toHaveBeenCalledTimes(1)
    })

    it('should not search when term is less than 3 characters', async () => {
      const onSearch = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          onSearch,
        }),
      )

      await act(async () => {
        result.current.handleInputChange('@ab', 3)
      })

      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 350))
      })

      expect(onSearch).not.toHaveBeenCalled()
    })

    it('should not search when autocomplete is not open', async () => {
      const onSearch = vi.fn()
      renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          onSearch,
        }),
      )

      // Don't trigger autocomplete opening, just wait
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 350))
      })

      expect(onSearch).not.toHaveBeenCalled()
    })

    it('should not search when disabled', async () => {
      const onSearch = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          enabled: false,
          onSearch,
        }),
      )

      await act(async () => {
        result.current.handleInputChange('@button', 7)
      })

      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 350))
      })

      expect(onSearch).not.toHaveBeenCalled()
    })

    it('should cancel previous timer on new search', async () => {
      const onSearch = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          onSearch,
        }),
      )

      // First search
      await act(async () => {
        result.current.handleInputChange('@button', 7)
      })

      // Second search before timer completes (wait only 100ms)
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 100))
      })

      await act(async () => {
        result.current.handleInputChange('@component', 10)
      })

      // Wait for the full debounce period
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 350))
      })

      // Should only search for the latest term
      expect(onSearch).toHaveBeenCalledTimes(1)
      expect(onSearch).toHaveBeenCalledWith('component')
    })

    it('should clean up timer on unmount', async () => {
      const onSearch = vi.fn()
      const { result, unmount } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          onSearch,
        }),
      )

      await act(async () => {
        result.current.handleInputChange('@button', 7)
      })

      unmount()

      // Wait for what would have been the debounce period
      await new Promise((resolve) => setTimeout(resolve, 350))

      // Should not call after unmount
      expect(onSearch).not.toHaveBeenCalled()
    })
  })

  describe('File Selection (selectFile)', () => {
    it('should replace @searchTerm with @filepath', () => {
      const onValueChange = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: 'Check @but in code',
          onValueChange,
          results: mockResults,
        }),
      )

      // Trigger autocomplete
      act(() => {
        result.current.handleInputChange('Check @but in code', 10)
      })

      // Select a file
      act(() => {
        result.current.selectFile(mockResults[0])
      })

      expect(onValueChange).toHaveBeenCalledWith('Check @src/components/button.tsx in code')
    })

    it('should close autocomplete after selection', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: '@but',
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@but', 4)
      })
      expect(result.current.isOpen).toBe(true)

      act(() => {
        result.current.selectFile(mockResults[0])
      })

      expect(result.current.isOpen).toBe(false)
    })

    it('should call onSelect callback with file path', () => {
      const onSelect = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: '@but',
          onSelect,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@but', 4)
      })

      act(() => {
        result.current.selectFile(mockResults[1])
      })

      expect(onSelect).toHaveBeenCalledWith('src/hooks/use-tasks.ts')
    })

    it('should handle selection at beginning of text', () => {
      const onValueChange = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: '@but',
          onValueChange,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@but', 4)
      })

      act(() => {
        result.current.selectFile(mockResults[0])
      })

      expect(onValueChange).toHaveBeenCalledWith('@src/components/button.tsx')
    })

    it('should handle selection in middle of text', () => {
      const onValueChange = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: 'Look at @but for examples',
          onValueChange,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('Look at @but for examples', 12)
      })

      act(() => {
        result.current.selectFile(mockResults[0])
      })

      expect(onValueChange).toHaveBeenCalledWith('Look at @src/components/button.tsx for examples')
    })
  })

  describe('Keyboard Navigation (handleKeyDown)', () => {
    const createKeyboardEvent = (key: string): React.KeyboardEvent<HTMLTextAreaElement> =>
      ({
        key,
        preventDefault: vi.fn(),
        stopPropagation: vi.fn(),
      }) as any

    it('should increase selectedIndex on ArrowDown', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      // Open autocomplete
      act(() => {
        result.current.handleInputChange('@comp', 5)
      })

      expect(result.current.selectedIndex).toBe(0)

      // Press ArrowDown
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
      })

      expect(result.current.selectedIndex).toBe(1)
    })

    it('should decrease selectedIndex on ArrowUp', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })

      // Move down first
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
      })

      expect(result.current.selectedIndex).toBe(2)

      // Press ArrowUp
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowUp'))
      })

      expect(result.current.selectedIndex).toBe(1)
    })

    it('should stop at last result when pressing ArrowDown', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })

      // Press ArrowDown many times
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
      })

      expect(result.current.selectedIndex).toBe(2) // Last index (0-based)
    })

    it('should stop at 0 when pressing ArrowUp', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })

      expect(result.current.selectedIndex).toBe(0)

      // Press ArrowUp
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowUp'))
      })

      expect(result.current.selectedIndex).toBe(0) // Should not go negative
    })

    it('should select file on Enter', () => {
      const onValueChange = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: '@comp',
          onValueChange,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })

      // Move to second item
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
      })

      // Press Enter
      const enterEvent = createKeyboardEvent('Enter')
      act(() => {
        result.current.handleKeyDown(enterEvent)
      })

      expect(enterEvent.preventDefault).toHaveBeenCalled()
      expect(onValueChange).toHaveBeenCalledWith('@src/hooks/use-tasks.ts')
    })

    it('should select file on Tab', () => {
      const onValueChange = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: '@comp',
          onValueChange,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })

      // Press Tab
      const tabEvent = createKeyboardEvent('Tab')
      act(() => {
        result.current.handleKeyDown(tabEvent)
      })

      expect(tabEvent.preventDefault).toHaveBeenCalled()
      expect(onValueChange).toHaveBeenCalledWith('@src/components/button.tsx')
    })

    it('should close autocomplete on Escape', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })
      expect(result.current.isOpen).toBe(true)

      // Press Escape
      const escapeEvent = createKeyboardEvent('Escape')
      act(() => {
        result.current.handleKeyDown(escapeEvent)
      })

      expect(escapeEvent.preventDefault).toHaveBeenCalled()
      expect(escapeEvent.stopPropagation).toHaveBeenCalled()
      expect(result.current.isOpen).toBe(false)
    })

    it('should do nothing when autocomplete is closed', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      // Don't open autocomplete
      expect(result.current.isOpen).toBe(false)

      // Press keys
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
      })

      // Should stay closed and not change state
      expect(result.current.isOpen).toBe(false)
      expect(result.current.selectedIndex).toBe(0)
    })

    it('should do nothing when no results', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: [],
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })
      expect(result.current.isOpen).toBe(true)

      // Press keys with no results
      act(() => {
        result.current.handleKeyDown(createKeyboardEvent('ArrowDown'))
      })

      expect(result.current.selectedIndex).toBe(0)
    })
  })

  describe('Props Integration', () => {
    it('should return provided results', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          results: mockResults,
        }),
      )

      expect(result.current.results).toEqual(mockResults)
    })

    it('should return isLoading state', () => {
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          isLoading: true,
        }),
      )

      expect(result.current.isLoading).toBe(true)
    })

    it('should return error state', () => {
      const error = new Error('Search failed')
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          error,
        }),
      )

      expect(result.current.error).toBe(error)
    })

    it('should work without onSelect callback', () => {
      const onValueChange = vi.fn()
      const { result } = renderHook(() =>
        useFileAutocomplete({
          ...defaultProps,
          value: '@comp',
          onValueChange,
          onSelect: undefined,
          results: mockResults,
        }),
      )

      act(() => {
        result.current.handleInputChange('@comp', 5)
      })

      // Should not throw when selecting
      act(() => {
        result.current.selectFile(mockResults[0])
      })

      expect(onValueChange).toHaveBeenCalled()
    })

    it('should provide inputRef', () => {
      const { result } = renderHook(() => useFileAutocomplete(defaultProps))

      expect(result.current.inputRef).toBeDefined()
      expect(result.current.inputRef.current).toBeNull() // Not attached to real element in test
    })
  })
})

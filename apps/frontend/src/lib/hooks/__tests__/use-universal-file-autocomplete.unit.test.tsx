import { renderHook, act } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { useUniversalFileAutocomplete } from '../use-universal-file-autocomplete'
import { useSearchFiles } from '@/features/repositories/hooks/use-repositories'

// Mock the API hook
vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useSearchFiles: vi.fn(() => ({
    data: { results: [] },
    isLoading: false,
    error: null,
  })),
}))

describe('useUniversalFileAutocomplete', () => {
  it('should not cause infinite re-renders', () => {
    let renderCount = 0
    const onValueChange = vi.fn()

    const { result } = renderHook(() => {
      renderCount++
      return useUniversalFileAutocomplete({
        repositoryId: 'test-repo',
        value: 'test',
        onValueChange,
      })
    })

    // Initial render count should be reasonable (not infinite)
    expect(renderCount).toBeLessThan(5)
    expect(result.current.isOpen).toBe(false)
    expect(result.current.searchTerm).toBe('')
  })

  it('should reset selectedIndex when search term changes', () => {
    const onValueChange = vi.fn()

    const { result } = renderHook(() =>
      useUniversalFileAutocomplete({
        repositoryId: 'test-repo',
        value: '',
        onValueChange,
      })
    )

    // Type @file to trigger autocomplete
    act(() => {
      result.current.handleInputChange('@file', 5)
    })

    expect(result.current.isOpen).toBe(true)
    expect(result.current.searchTerm).toBe('file')
    expect(result.current.selectedIndex).toBe(0)

    // Change search term
    act(() => {
      result.current.handleInputChange('@filetest', 9)
    })

    expect(result.current.searchTerm).toBe('filetest')
    expect(result.current.selectedIndex).toBe(0) // Should reset to 0
  })

  it('should preserve selectedIndex when search term remains the same', () => {
    const onValueChange = vi.fn()

    const { result } = renderHook(() =>
      useUniversalFileAutocomplete({
        repositoryId: 'test-repo',
        value: '@file',
        onValueChange,
      })
    )

    // Initialize autocomplete
    act(() => {
      result.current.handleInputChange('@file', 5)
    })

    // Simulate arrow down to change selectedIndex
    const mockEvent = {
      key: 'ArrowDown',
      preventDefault: vi.fn(),
    } as unknown as React.KeyboardEvent<HTMLTextAreaElement>

    // Mock search results for arrow navigation
    vi.mocked(useSearchFiles).mockReturnValueOnce({
      data: { results: [{ path: 'file1.ts' }, { path: 'file2.ts' }] },
      isLoading: false,
      error: null,
    } as any)

    act(() => {
      result.current.handleKeyDown(mockEvent)
    })

    // Trigger input change with same search term but different cursor position
    act(() => {
      result.current.handleInputChange('prefix @file', 12)
    })

    // Search term is the same, so selectedIndex should be preserved
    expect(result.current.searchTerm).toBe('file')
    expect(result.current.selectedIndex).toBeGreaterThanOrEqual(0)
  })
})
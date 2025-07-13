import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useInspector } from '../use-inspector'
import { PreviewProvider } from '../use-preview'
import { createMockElementInfo } from './test-helpers'

// Mock clipboard API
const mockWriteText = vi.fn()
Object.defineProperty(navigator, 'clipboard', {
  value: {
    writeText: mockWriteText,
  },
  writable: true,
})

function createWrapper() {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <PreviewProvider>{children}</PreviewProvider>
  }
}

describe('useInspector', () => {
  let wrapper: ReturnType<typeof createWrapper>

  beforeEach(() => {
    wrapper = createWrapper()
    vi.clearAllMocks()
    mockWriteText.mockResolvedValue(undefined)
  })

  describe('Inspector State', () => {
    it('should return correct initial state', () => {
      const { result } = renderHook(() => useInspector(), { wrapper })

      expect(result.current.isActive).toBe(false)
      expect(result.current.selectedElement).toBe(null)
      expect(typeof result.current.toggle).toBe('function')
      expect(typeof result.current.selectElement).toBe('function')
      expect(typeof result.current.clearSelection).toBe('function')
    })
  })

  describe('Inspector Toggle', () => {
    it('should toggle inspector state', () => {
      const { result } = renderHook(() => useInspector(), { wrapper })

      expect(result.current.isActive).toBe(false)

      act(() => {
        result.current.toggle()
      })

      expect(result.current.isActive).toBe(true)

      act(() => {
        result.current.toggle()
      })

      expect(result.current.isActive).toBe(false)
    })
  })

  describe('Element Selection', () => {
    it('should select element and copy selector to clipboard', async () => {
      const { result } = renderHook(() => useInspector(), { wrapper })
      const mockElement = createMockElementInfo({
        attributes: { class: 'test-element', id: 'test' },
        boundingRect: { top: 10, left: 20, width: 100, height: 50, x: 20, y: 10 },
      })

      act(() => {
        result.current.selectElement(mockElement)
      })

      expect(result.current.selectedElement).toEqual(mockElement)
      expect(mockWriteText).toHaveBeenCalledWith('.test-element')
    })

    it('should handle element selection with null', () => {
      const { result } = renderHook(() => useInspector(), { wrapper })

      act(() => {
        result.current.selectElement(null)
      })

      expect(result.current.selectedElement).toBe(null)
      expect(mockWriteText).not.toHaveBeenCalled()
    })

    it('should not copy to clipboard when element has no selector', () => {
      const { result } = renderHook(() => useInspector(), { wrapper })
      const mockElement = createMockElementInfo({
        selector: '',
        boundingRect: { top: 10, left: 20, width: 100, height: 50, x: 20, y: 10 },
      })

      act(() => {
        result.current.selectElement(mockElement)
      })

      expect(result.current.selectedElement).toEqual(mockElement)
      expect(mockWriteText).not.toHaveBeenCalled()
    })

    it('should handle clipboard errors gracefully', () => {
      const { result } = renderHook(() => useInspector(), { wrapper })
      const mockElement = createMockElementInfo({
        boundingRect: { top: 10, left: 20, width: 100, height: 50, x: 20, y: 10 },
      })

      // Mock clipboard to reject
      mockWriteText.mockRejectedValue(new Error('Clipboard error'))

      act(() => {
        result.current.selectElement(mockElement)
      })

      expect(result.current.selectedElement).toEqual(mockElement)
      expect(mockWriteText).toHaveBeenCalledWith('.test-element')
    })
  })

  describe('Clear Selection', () => {
    it('should clear selected element', () => {
      const { result } = renderHook(() => useInspector(), { wrapper })
      const mockElement = createMockElementInfo({
        boundingRect: { top: 10, left: 20, width: 100, height: 50, x: 20, y: 10 },
      })

      // First select an element
      act(() => {
        result.current.selectElement(mockElement)
      })

      expect(result.current.selectedElement).toEqual(mockElement)

      // Then clear selection
      act(() => {
        result.current.clearSelection()
      })

      expect(result.current.selectedElement).toBe(null)
    })
  })

  describe('Integration with Preview Context', () => {
    it('should reflect inspector state changes from context', () => {
      const { result } = renderHook(
        () => {
          const inspector = useInspector()
          const preview = useInspector() // This will share the same context
          return { inspector, preview }
        },
        { wrapper },
      )

      expect(result.current.inspector.isActive).toBe(false)

      act(() => {
        result.current.inspector.toggle()
      })

      expect(result.current.inspector.isActive).toBe(true)
      expect(result.current.preview.isActive).toBe(true)
    })

    it('should reflect element selection changes from context', () => {
      const { result } = renderHook(
        () => {
          const inspector1 = useInspector()
          const inspector2 = useInspector() // Same context
          return { inspector1, inspector2 }
        },
        { wrapper },
      )

      const mockElement = createMockElementInfo({
        boundingRect: { top: 10, left: 20, width: 100, height: 50, x: 20, y: 10 },
      })

      act(() => {
        result.current.inspector1.selectElement(mockElement)
      })

      expect(result.current.inspector1.selectedElement).toEqual(mockElement)
      expect(result.current.inspector2.selectedElement).toEqual(mockElement)
    })
  })
})

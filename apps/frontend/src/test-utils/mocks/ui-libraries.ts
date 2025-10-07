import type { ReactNode } from 'react'
import { vi } from 'vitest'

/**
 * Complex UI component mocks for testing
 * Note: These mock components return the module structure expected by vi.mock()
 */

interface MockComponentProps {
  children?: ReactNode
}

interface MockDialogProps extends MockComponentProps {
  open?: boolean
}

interface MockTabsContentProps extends MockComponentProps {
  value?: string
  selectedValue?: string
}

interface MockCodeEditorProps {
  value?: string
  onChange?: (value: string) => void
}

/**
 * Mock resizable panel components
 */
export const mockResizableComponents = () => ({
  ResizablePanel: vi.fn(({ children }: MockComponentProps) => children),
  ResizablePanelGroup: vi.fn(({ children }: MockComponentProps) => children),
  ResizableHandle: vi.fn(() => null),
})

/**
 * Mock dialog components
 */
export const mockDialogComponents = () => ({
  Dialog: vi.fn(({ children, open }: MockDialogProps) => (open ? children : null)),
  DialogTrigger: vi.fn(({ children }: MockComponentProps) => children),
  DialogContent: vi.fn(({ children }: MockComponentProps) => children),
  DialogHeader: vi.fn(({ children }: MockComponentProps) => children),
  DialogTitle: vi.fn(({ children }: MockComponentProps) => children),
  DialogDescription: vi.fn(({ children }: MockComponentProps) => children),
  DialogFooter: vi.fn(({ children }: MockComponentProps) => children),
})

/**
 * Mock tooltip components
 */
export const mockTooltipComponents = () => ({
  Tooltip: vi.fn(({ children }: MockComponentProps) => children),
  TooltipTrigger: vi.fn(({ children }: MockComponentProps) => children),
  TooltipContent: vi.fn(({ children }: MockComponentProps) => children),
  TooltipProvider: vi.fn(({ children }: MockComponentProps) => children),
})

/**
 * Mock select/dropdown components
 */
export const mockSelectComponents = () => ({
  Select: vi.fn(({ children }: MockComponentProps) => children),
  SelectTrigger: vi.fn(({ children }: MockComponentProps) => children),
  SelectContent: vi.fn(({ children }: MockComponentProps) => children),
  SelectItem: vi.fn(({ children }: MockComponentProps) => children),
  SelectValue: vi.fn(() => null),
})

/**
 * Mock tabs components
 */
export const mockTabsComponents = () => ({
  Tabs: vi.fn(({ children }: MockComponentProps) => children),
  TabsList: vi.fn(({ children }: MockComponentProps) => children),
  TabsTrigger: vi.fn(({ children }: MockComponentProps) => children),
  TabsContent: vi.fn(({ children, value, selectedValue }: MockTabsContentProps) =>
    selectedValue === value ? children : null,
  ),
})

/**
 * Mock scroll area component
 */
export const mockScrollAreaComponents = () => ({
  ScrollArea: vi.fn(({ children }: MockComponentProps) => children),
  ScrollBar: vi.fn(() => null),
})

/**
 * Mock code editor components
 */
export const mockCodeEditorComponents = () => ({
  default: vi.fn(({ value, onChange }: MockCodeEditorProps) => ({
    value,
    onChange,
  })),
})

/**
 * Create a mock module for UI libraries
 */
export function createUILibraryMocks() {
  return {
    '@/lib/components/ui/resizable': mockResizableComponents(),
    '@/lib/components/ui/dialog': mockDialogComponents(),
    '@/lib/components/ui/tooltip': mockTooltipComponents(),
    '@/lib/components/ui/select': mockSelectComponents(),
    '@/lib/components/ui/tabs': mockTabsComponents(),
    '@/lib/components/ui/scroll-area': mockScrollAreaComponents(),
    '@uiw/react-codemirror': mockCodeEditorComponents(),
  }
}

/**
 * Helper to mock all UI libraries at once
 * Note: vi.mock() calls must be hoisted and can't be called dynamically
 * Use createUILibraryMocks() instead and mock each library individually in your test file
 */
export function mockAllUILibraries() {
  throw new Error(
    'mockAllUILibraries() cannot be used because vi.mock() must be hoisted. Use createUILibraryMocks() instead.',
  )
}

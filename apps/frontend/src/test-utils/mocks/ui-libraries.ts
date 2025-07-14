import { vi } from 'vitest'

/**
 * Complex UI component mocks for testing
 * Note: These mock components return the module structure expected by vi.mock()
 */

/**
 * Mock resizable panel components
 */
export const mockResizableComponents = () => ({
  ResizablePanel: vi.fn(({ children }: any) => children),
  ResizablePanelGroup: vi.fn(({ children }: any) => children),
  ResizableHandle: vi.fn(() => null),
})

/**
 * Mock dialog components
 */
export const mockDialogComponents = () => ({
  Dialog: vi.fn(({ children, open }: any) => (open ? children : null)),
  DialogTrigger: vi.fn(({ children }: any) => children),
  DialogContent: vi.fn(({ children }: any) => children),
  DialogHeader: vi.fn(({ children }: any) => children),
  DialogTitle: vi.fn(({ children }: any) => children),
  DialogDescription: vi.fn(({ children }: any) => children),
  DialogFooter: vi.fn(({ children }: any) => children),
})

/**
 * Mock tooltip components
 */
export const mockTooltipComponents = () => ({
  Tooltip: vi.fn(({ children }: any) => children),
  TooltipTrigger: vi.fn(({ children }: any) => children),
  TooltipContent: vi.fn(({ children }: any) => children),
  TooltipProvider: vi.fn(({ children }: any) => children),
})

/**
 * Mock select/dropdown components
 */
export const mockSelectComponents = () => ({
  Select: vi.fn(({ children }: any) => children),
  SelectTrigger: vi.fn(({ children }: any) => children),
  SelectContent: vi.fn(({ children }: any) => children),
  SelectItem: vi.fn(({ children }: any) => children),
  SelectValue: vi.fn(() => null),
})

/**
 * Mock tabs components
 */
export const mockTabsComponents = () => ({
  Tabs: vi.fn(({ children }: any) => children),
  TabsList: vi.fn(({ children }: any) => children),
  TabsTrigger: vi.fn(({ children }: any) => children),
  TabsContent: vi.fn(({ children, value, selectedValue }: any) =>
    selectedValue === value ? children : null,
  ),
})

/**
 * Mock scroll area component
 */
export const mockScrollAreaComponents = () => ({
  ScrollArea: vi.fn(({ children }: any) => children),
  ScrollBar: vi.fn(() => null),
})

/**
 * Mock code editor components
 */
export const mockCodeEditorComponents = () => ({
  default: vi.fn(({ value, onChange }: any) => ({
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
 */
export function mockAllUILibraries() {
  const mocks = createUILibraryMocks()

  Object.entries(mocks).forEach(([moduleName, exports]) => {
    vi.mock(moduleName, () => exports)
  })
}

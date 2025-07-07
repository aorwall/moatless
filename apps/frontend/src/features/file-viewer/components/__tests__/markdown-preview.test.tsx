import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import mermaid from 'mermaid'
import { MarkdownPreview } from '../markdown-preview'

// Mock mermaid module
vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(),
  },
}))

// Mock theme provider
const mockUseTheme = vi.fn(() => ({ theme: 'light' }))
vi.mock('@/lib/components/theme-provider', () => ({
  useTheme: () => mockUseTheme(),
}))

// Mock Dialog components
vi.mock('@/lib/components/ui/dialog', () => ({
  Dialog: ({ children, open }: any) => open ? <div data-testid="dialog">{children}</div> : null,
  DialogContent: ({ children }: any) => <div data-testid="dialog-content">{children}</div>,
  DialogHeader: ({ children }: any) => <div>{children}</div>,
  DialogTitle: ({ children }: any) => <h2>{children}</h2>,
}))

describe('MarkdownPreview', () => {
  beforeEach(() => {
    // Reset all mocks before each test
    vi.clearAllMocks()
    
    // Reset mermaid render implementation to default success
    vi.mocked(mermaid.render).mockImplementation((id, graphDefinition) => {
      return Promise.resolve({
        svg: `<svg id="${id}">${graphDefinition}</svg>`,
        bindFunctions: vi.fn(),
      } as any)
    })
  })

  afterEach(() => {
    // Clean up DOM after each test
    document.body.innerHTML = ''
  })

  describe('Basic Markdown Rendering', () => {
    it('should render plain text', () => {
      render(<MarkdownPreview content="Hello World" />)
      expect(screen.getByText('Hello World')).toBeInTheDocument()
    })

    it('should render headings', () => {
      render(<MarkdownPreview content="# Heading 1" />)
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Heading 1')
      
      // Test level 2 heading separately
      const { container } = render(<MarkdownPreview content="## Heading 2" />)
      const h2 = container.querySelector('h2')
      expect(h2).toBeInTheDocument()
      expect(h2).toHaveTextContent('Heading 2')
    })

    it('should render bold and italic text', () => {
      render(<MarkdownPreview content="**bold** and *italic*" />)
      expect(screen.getByText('bold')).toBeInTheDocument()
      expect(screen.getByText('italic')).toBeInTheDocument()
    })

    it('should render code blocks', () => {
      const content = '```javascript\nconst hello = "world";\n```'
      render(<MarkdownPreview content={content} />)
      expect(screen.getByText('const hello = "world";')).toBeInTheDocument()
    })

    it('should render inline code', () => {
      render(<MarkdownPreview content="Use `npm install` to install dependencies" />)
      expect(screen.getByText('npm install')).toBeInTheDocument()
    })

    it('should render tables', () => {
      const content = `
| Header 1 | Header 2 |
|----------|----------|
| Cell 1   | Cell 2   |
`
      render(<MarkdownPreview content={content} />)
      expect(screen.getByText('Header 1')).toBeInTheDocument()
      expect(screen.getByText('Cell 1')).toBeInTheDocument()
    })

    it('should render images with proper attributes', () => {
      const content = '![Alt text](https://example.com/image.png)'
      render(<MarkdownPreview content={content} />)
      const img = screen.getByRole('img', { name: 'Alt text' })
      expect(img).toHaveAttribute('src', 'https://example.com/image.png')
      expect(img).toHaveClass('max-w-full', 'h-auto')
    })
  })

  describe('Mermaid Diagram Rendering - Success Cases', () => {
    it('should render a simple mermaid flowchart', async () => {
      const content = `
\`\`\`mermaid
graph TD
    A[Start] --> B[End]
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalledWith(
          expect.stringMatching(/^mermaid-\d+-0-[a-z0-9]+$/),
          expect.stringContaining('graph TD')
        )
      })

      // Check that the mermaid diagram was rendered
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(1)
        expect(mermaidElements[0]).toContainHTML('<svg')
      })
    })

    it('should render multiple mermaid diagrams', async () => {
      const content = `
\`\`\`mermaid
graph TD
    A[First] --> B[Diagram]
\`\`\`

Some text between diagrams

\`\`\`mermaid
sequenceDiagram
    Alice->>Bob: Hello
    Bob->>Alice: Hi
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for both diagrams to be processed
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(2)
      })
      
      // Check that both diagrams were rendered by looking at the arguments
      const calls = vi.mocked(mermaid.render).mock.calls
      const graphDefinitions = calls.map(call => call[1])
      expect(graphDefinitions.some(def => def.includes('A[First] --> B[Diagram]'))).toBe(true)
      expect(graphDefinitions.some(def => def.includes('Alice->>Bob: Hello'))).toBe(true)
    })

    it('should initialize mermaid with correct theme', () => {
      render(<MarkdownPreview content="# Test" />)
      
      expect(mermaid.initialize).toHaveBeenCalledWith({
        theme: 'default',
        startOnLoad: false,
        fontFamily: 'ui-monospace, monospace',
        securityLevel: 'strict',
        logLevel: 'error',
      })
    })
  })

  describe('Mermaid Diagram Rendering - Fail States', () => {
    it('should handle mermaid syntax errors gracefully', async () => {
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
      
      // Mock mermaid.render to reject with syntax error
      vi.mocked(mermaid.render).mockRejectedValueOnce(
        new Error('Parse error on line 2: Expecting "GRAPH", got "INVALID"')
      )

      const content = `
\`\`\`mermaid
INVALID SYNTAX HERE
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })

      // Check error message is displayed
      await waitFor(() => {
        const errorElement = screen.getByText(/Error rendering diagram:/)
        expect(errorElement).toBeInTheDocument()
        expect(errorElement).toHaveTextContent('Parse error on line 2')
      })

      expect(consoleErrorSpy).toHaveBeenCalledWith(
        'Mermaid rendering error:',
        expect.any(Error)
      )

      consoleErrorSpy.mockRestore()
    })

    it('should handle multiple failing diagrams', async () => {
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
      
      // Mock all mermaid.render calls to fail
      vi.mocked(mermaid.render)
        .mockRejectedValueOnce(new Error('Error in diagram 1'))
        .mockRejectedValueOnce(new Error('Error in diagram 2'))

      const content = `
\`\`\`mermaid
INVALID 1
\`\`\`

\`\`\`mermaid
INVALID 2
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for mermaid processing to complete
      await waitFor(() => {
        const errorElements = document.querySelectorAll('.text-destructive')
        expect(errorElements.length).toBeGreaterThan(0)
      })

      // Check that errors are displayed (may only show one due to React's error boundary)
      const errorText = screen.getByText(/Error rendering diagram:/)
      expect(errorText).toBeInTheDocument()

      consoleErrorSpy.mockRestore()
    })

    it('should handle mermaid initialization errors', async () => {
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
      
      // Mock mermaid.render to reject (async error)
      vi.mocked(mermaid.render).mockRejectedValueOnce(
        new Error('Mermaid not initialized properly')
      )

      const content = `
\`\`\`mermaid
graph TD
    A --> B
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })

      // Error should be caught and displayed
      await waitFor(() => {
        const errorElement = screen.getByText(/Error rendering diagram:/)
        expect(errorElement).toHaveTextContent('Mermaid not initialized properly')
      })

      consoleErrorSpy.mockRestore()
    })

    it('should handle empty mermaid code blocks', async () => {
      const content = `
\`\`\`mermaid
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalledWith(
          expect.any(String),
          '' // Empty content
        )
      })
    })

    it('should re-render when content changes', async () => {
      const { rerender } = render(<MarkdownPreview content="# Initial" />)

      expect(mermaid.render).not.toHaveBeenCalled()

      const newContent = `
\`\`\`mermaid
graph TD
    A --> B
\`\`\`
`
      rerender(<MarkdownPreview content={newContent} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })
    })

    it('should handle very large mermaid diagrams', async () => {
      // Create a large diagram with many nodes
      const largeContent = `
\`\`\`mermaid
graph TD
${Array.from({ length: 100 }, (_, i) => `    A${i} --> B${i}`).join('\n')}
\`\`\`
`
      render(<MarkdownPreview content={largeContent} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })

      // Should still render successfully
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(1)
      })
    })

    it('should handle special characters in mermaid diagrams', async () => {
      const content = `
\`\`\`mermaid
graph TD
    A["<Special> & 'Characters'"] --> B[Normal]
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })
    })
  })

  describe('Theme Support', () => {
    it('should use dark theme when theme is dark', () => {
      mockUseTheme.mockReturnValue({ theme: 'dark' })
      
      render(<MarkdownPreview content="# Test" />)
      
      expect(mermaid.initialize).toHaveBeenCalledWith(
        expect.objectContaining({
          theme: 'dark',
        })
      )
    })

    it('should use dark theme when system prefers dark', () => {
      mockUseTheme.mockReturnValue({ theme: 'system' })
      
      // Mock matchMedia to return dark preference
      Object.defineProperty(window, 'matchMedia', {
        writable: true,
        value: vi.fn().mockImplementation(query => ({
          matches: query === '(prefers-color-scheme: dark)',
          media: query,
          onchange: null,
          addListener: vi.fn(),
          removeListener: vi.fn(),
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
          dispatchEvent: vi.fn(),
        })),
      })
      
      render(<MarkdownPreview content="# Test" />)
      
      expect(mermaid.initialize).toHaveBeenCalledWith(
        expect.objectContaining({
          theme: 'dark',
        })
      )
    })
  })

  describe('Memory Leaks and Cleanup', () => {
    it('should process mermaid diagrams on every content change', async () => {
      const content = `
\`\`\`mermaid
graph TD
    A --> B
\`\`\`
`
      const { rerender } = render(<MarkdownPreview content={content} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })

      // Re-render with same content - React effect will run again
      rerender(<MarkdownPreview content={content} />)

      // Should be called again because the effect runs on every render due to content dependency
      await waitFor(() => {
        // At least 2 calls expected (initial + re-render)
        expect(vi.mocked(mermaid.render).mock.calls.length).toBeGreaterThanOrEqual(2)
      })
    })

    it('should handle rapid content changes', async () => {
      const { rerender } = render(<MarkdownPreview content="Initial" />)

      // Rapidly change content multiple times
      for (let i = 0; i < 5; i++) {
        const content = `
\`\`\`mermaid
graph TD
    A${i} --> B${i}
\`\`\`
`
        rerender(<MarkdownPreview content={content} />)
      }

      // Should handle all changes without errors
      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })
    })
  })

  describe('Edge Cases', () => {
    it('should handle mermaid blocks with extra whitespace', async () => {
      const content = `
\`\`\`mermaid    

    graph TD
        A --> B
    
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      await waitFor(() => {
        expect(mermaid.render).toHaveBeenCalled()
      })
    })

    it('should handle mixed content with mermaid and other code blocks', async () => {
      const content = `
# Documentation

\`\`\`javascript
const code = "example";
\`\`\`

\`\`\`mermaid
graph TD
    A --> B
\`\`\`

\`\`\`python
print("hello")
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for mermaid block to be processed
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(1)
      })

      // Other code blocks should remain as is
      expect(screen.getByText('const code = "example";')).toBeInTheDocument()
      expect(screen.getByText('print("hello")')).toBeInTheDocument()
    })

    it('should handle null or undefined content', () => {
      const { rerender } = render(<MarkdownPreview content="" />)
      
      // Should not crash with null/undefined
      rerender(<MarkdownPreview content={null as any} />)
      rerender(<MarkdownPreview content={undefined as any} />)
    })
  })

  describe('Mermaid Diagram Expansion', () => {
    it('should make mermaid diagrams clickable', async () => {
      const content = `
\`\`\`mermaid
graph TD
    A[Click me] --> B[To expand]
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for mermaid to render
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(1)
      })

      // Check that the diagram has a clickable wrapper
      const wrapper = document.querySelector('.relative.group.cursor-pointer')
      expect(wrapper).toBeInTheDocument()
      
      // Check for the expand button
      const expandButton = screen.getByLabelText('Expand diagram')
      expect(expandButton).toBeInTheDocument()
    })

    it('should open dialog when mermaid diagram is clicked', async () => {
      const content = `
\`\`\`mermaid
graph TD
    A[Start] --> B[End]
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for mermaid to render
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(1)
      })

      // Click the diagram wrapper
      const wrapper = document.querySelector('.relative.group.cursor-pointer') as HTMLElement
      expect(wrapper).toBeInTheDocument()
      fireEvent.click(wrapper)

      // Check that dialog opened
      await waitFor(() => {
        expect(screen.getByTestId('dialog')).toBeInTheDocument()
        expect(screen.getByTestId('dialog-content')).toBeInTheDocument()
        expect(screen.getByText('Mermaid Diagram')).toBeInTheDocument()
      })
    })

    it('should display expanded SVG in dialog', async () => {
      const content = `
\`\`\`mermaid
graph LR
    A[Expanded] --> B[View]
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for mermaid to render
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(1)
      })

      // Click the diagram
      const wrapper = document.querySelector('.relative.group.cursor-pointer') as HTMLElement
      fireEvent.click(wrapper)

      // Check that the SVG is displayed in the dialog
      await waitFor(() => {
        const dialogContent = screen.getByTestId('dialog-content')
        const svg = dialogContent.querySelector('svg')
        expect(svg).toBeInTheDocument()
        expect(dialogContent.innerHTML).toContain('graph LR')
      })
    })

    it('should show expand button on hover', async () => {
      const content = `
\`\`\`mermaid
flowchart TB
    A[Hover] --> B[Me]
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for mermaid to render
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(1)
      })

      // The expand button should have opacity classes
      const expandButton = screen.getByLabelText('Expand diagram')
      expect(expandButton.className).toContain('opacity-0')
      expect(expandButton.className).toContain('group-hover:opacity-100')
    })

    it('should handle multiple diagrams independently', async () => {
      const content = `
\`\`\`mermaid
graph TD
    A1[First] --> B1[Diagram]
\`\`\`

Some text

\`\`\`mermaid
graph TD
    A2[Second] --> B2[Diagram]
\`\`\`
`
      render(<MarkdownPreview content={content} />)

      // Wait for both diagrams to render
      await waitFor(() => {
        const mermaidElements = document.querySelectorAll('.mermaid-rendered')
        expect(mermaidElements).toHaveLength(2)
      })

      // Both should be clickable
      const wrappers = document.querySelectorAll('.relative.group.cursor-pointer')
      expect(wrappers).toHaveLength(2)

      // Click the first diagram
      fireEvent.click(wrappers[0] as HTMLElement)

      // Check that dialog shows first diagram content
      await waitFor(() => {
        const dialogContent = screen.getByTestId('dialog-content')
        expect(dialogContent.innerHTML).toContain('A1[First]')
        expect(dialogContent.innerHTML).not.toContain('A2[Second]')
      })
    })
  })
})
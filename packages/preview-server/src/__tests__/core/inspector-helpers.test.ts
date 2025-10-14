import { describe, expect, it } from 'vitest'
import { formatSourceLocation, generateComponentList } from '../../core/inspector-helpers.js'
import type { ElementInfo } from '../../types/index.js'

describe('formatSourceLocation', () => {
  it('should format complete source location', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [],
    }

    expect(formatSourceLocation(element)).toBe('src/components/Button.tsx:10:5')
  })

  it('should handle missing line number', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: null,
      columnNumber: 5,
      elementTag: 'button',
      parents: [],
    }

    expect(formatSourceLocation(element)).toBe('src/components/Button.tsx:0:5')
  })

  it('should handle missing column number', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: null,
      elementTag: 'button',
      parents: [],
    }

    expect(formatSourceLocation(element)).toBe('src/components/Button.tsx:10:0')
  })

  it('should handle missing both line and column', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: null,
      columnNumber: null,
      elementTag: 'button',
      parents: [],
    }

    expect(formatSourceLocation(element)).toBe('src/components/Button.tsx:0:0')
  })

  it('should handle missing filePath', () => {
    const element: ElementInfo = {
      filePath: null,
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [],
    }

    expect(formatSourceLocation(element)).toBe('No source location available')
  })
})

// hasSourceLocation tests removed - function no longer exists since path, lineNumber, and file are now required fields

describe('generateComponentList', () => {
  it('should generate list for element with source location', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [],
    }

    const result = generateComponentList(element)

    expect(result).toContain('Relevant components for this task:')
    expect(result).toContain('1. <button> in @src/components/Button.tsx:10')
  })

  it('should generate list with parent hierarchy', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [
        {
          filePath: 'src/components/Card.tsx',
          lineNumber: 5,
          columnNumber: 2,
          elementTag: 'div',
        },
        {
          filePath: 'src/App.tsx',
          lineNumber: 20,
          columnNumber: 1,
          elementTag: 'main',
        },
      ],
    }

    const result = generateComponentList(element)

    expect(result).toContain('1. <button> in @src/components/Button.tsx:10')
    expect(result).toContain('2. <div> in @src/components/Card.tsx:5')
    expect(result).toContain('3. <main> in @src/App.tsx:20')
  })

  it('should handle element without filePath', () => {
    const element: ElementInfo = {
      filePath: null,
      lineNumber: null,
      columnNumber: null,
      elementTag: 'button',
      parents: [],
    }

    const result = generateComponentList(element)

    expect(result).toBe(
      'No component hierarchy available. Enable code-inspector plugin to see file locations.',
    )
  })

  it('should skip parents without filePath', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [
        {
          filePath: null,
          lineNumber: null,
          columnNumber: null,
          elementTag: 'div',
        },
        {
          filePath: 'src/App.tsx',
          lineNumber: 20,
          columnNumber: 1,
          elementTag: 'main',
        },
      ],
    }

    const result = generateComponentList(element)

    expect(result).toContain('1. <button> in @src/components/Button.tsx:10')
    // Parent index starts from the first valid parent after button (index 0 is skipped, so next is 2)
    expect(result).toContain('<main> in @src/App.tsx:20')
    expect(result).not.toContain('<div>')
  })

  it('should handle empty parents array', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [],
    }

    const result = generateComponentList(element)

    expect(result).toContain('1. <button> in @src/components/Button.tsx:10')
    expect(result.split('\n').length).toBe(3) // Header + empty line + 1 component
  })

  it('should handle parents with lineNumber 0', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [
        {
          filePath: 'src/App.tsx',
          lineNumber: 0,
          columnNumber: 0,
          elementTag: 'div',
        },
      ],
    }

    const result = generateComponentList(element)

    expect(result).toContain('2. <div> in @src/App.tsx:0')
  })

  it('should handle parent with unknown elementTag', () => {
    const element: ElementInfo = {
      filePath: 'src/components/Button.tsx',
      lineNumber: 10,
      columnNumber: 5,
      elementTag: 'button',
      parents: [
        {
          filePath: 'src/App.tsx',
          lineNumber: 20,
          columnNumber: 1,
          elementTag: null,
        },
      ],
    }

    const result = generateComponentList(element)

    expect(result).toContain('2. <unknown> in @src/App.tsx:20')
  })
})

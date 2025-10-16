import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createFigmaClipboardParser, defaultPreset, generateFigmaPrompt } from '@moatless/figma'
import { describe, expect, it } from 'vitest'

// Create temp directory for prompt outputs
const TEMP_DIR = '/tmp/figma-prompts-test'
mkdirSync(TEMP_DIR, { recursive: true })
console.log(`\n📁 Saving prompts to: ${TEMP_DIR}\n`)

describe('Figma Prompt Transformer Integration', () => {
  it('should transform sample import data to prompt', () => {
    const importData = {
      nodeMetadata: {
        type: 'COMPONENT',
        name: 'context menu',
        absoluteBoundingBox: { width: 320, height: 420 },
        layoutMode: 'VERTICAL',
        itemSpacing: 12,
        paddingTop: 8,
        paddingRight: 8,
        paddingBottom: 8,
        paddingLeft: 8,
        children: [
          {
            type: 'FRAME',
            name: 'Header',
            layoutMode: 'HORIZONTAL',
            itemSpacing: 8,
            paddingTop: 12,
            paddingRight: 16,
            paddingBottom: 12,
            paddingLeft: 16,
            fills: [
              {
                type: 'SOLID',
                color: { r: 1, g: 1, b: 1, a: 1 },
              },
            ],
            absoluteBoundingBox: { width: 320, height: 48 },
          },
          {
            type: 'INSTANCE',
            name: 'menu item',
            componentId: '123:456',
            absoluteBoundingBox: { width: 320, height: 40 },
            componentProperties: {
              State: { type: 'VARIANT', value: 'default' },
              Label: { type: 'TEXT', value: 'Back' },
              LeftIcon: { type: 'BOOLEAN', value: true },
            },
          },
        ],
      },
      fileName: '@shadcn/ui - Design System (Community)',
      nodeName: 'context menu',
    }

    const prompt = generateFigmaPrompt(
      importData.nodeMetadata,
      importData.fileName,
      importData.nodeName,
      defaultPreset,
    )

    // Save prompt to file for inspection
    const outputPath = join(TEMP_DIR, 'figma-import-shadcn-prompt.txt')
    writeFileSync(outputPath, prompt, 'utf-8')
    console.log(`✅ Saved import prompt to: ${outputPath}`)

    // Verify prompt structure
    expect(prompt).toBeDefined()
    expect(prompt.length).toBeGreaterThan(300)

    // Verify header
    expect(prompt).toContain('Figma Design:')

    // Verify it contains expected node types
    expect(prompt).toContain('COMPONENT')
    expect(prompt).toContain('FRAME')
    expect(prompt).toContain('INSTANCE')

    // Verify it contains layout properties
    expect(prompt).toContain('Layout:')
    expect(prompt).toContain('Horizontal')
    expect(prompt).toContain('Vertical')
    expect(prompt).toContain('gap-')

    // Verify it contains fills
    expect(prompt).toContain('Fill:')
    expect(prompt).toContain('#FFFFFF')

    // Verify it contains dimensions
    expect(prompt).toContain('320x')

    // Verify hierarchical structure with arrows
    expect(prompt).toContain('→')
    expect(prompt).toContain('Children:')

    // Log sample of the prompt for debugging
    const lines = prompt.split('\n')
    console.log(`\n📝 Generated prompt (${lines.length} lines, ${prompt.length} chars)`)
    console.log('First 10 lines:')
    console.log(lines.slice(0, 10).join('\n'))
  })

  it('should transform figma-clipboard-shadcn.html to prompt', async () => {
    // Parse clipboard HTML
    const parser = createFigmaClipboardParser()

    const clipboardPath = join(__dirname, '../../services/__tests__/fixtures/figma-clipboard.html')
    const clipboardHtml = readFileSync(clipboardPath, 'utf-8')
    const parseResult = parser.parseClipboardHTML(clipboardHtml)

    // Transform to prompt
    const prompt = generateFigmaPrompt(
      parseResult.root,
      parseResult.root.name,
      parseResult.root.name,
      defaultPreset,
    )

    // Save prompt to file for inspection
    const outputPath = join(TEMP_DIR, 'figma-clipboard-shadcn-prompt.txt')
    writeFileSync(outputPath, prompt, 'utf-8')
    console.log(`✅ Saved clipboard prompt to: ${outputPath}`)

    // Verify prompt structure
    expect(prompt).toBeDefined()
    expect(prompt.length).toBeGreaterThan(1000)
    expect(prompt).toContain('DOCUMENT')
    expect(prompt).toContain('Layout:')
    expect(prompt).toContain('Fill:')
  })

  it('should handle node tree with layout properties', () => {
    const sampleNode = {
      guid: '0:0',
      type: 'FRAME',
      name: 'Test Frame',
      children: [],
      properties: {
        size: { x: 100, y: 200 },
        stackMode: 'HORIZONTAL',
        stackSpacing: 16,
        fillPaints: [
          {
            type: 'SOLID',
            color: { r: 1, g: 0, b: 0, a: 1 },
            visible: true,
          },
        ],
      },
    }

    const prompt = generateFigmaPrompt(sampleNode, 'Test File', 'Test Node', defaultPreset)

    expect(prompt).toContain('FRAME "Test Frame"')
    expect(prompt).toContain('100x200px')
    expect(prompt).toContain('Horizontal')
    expect(prompt).toContain('gap-4')
    expect(prompt).toContain('Fill: #FF0000')
  })

  it('should handle nested children', () => {
    const nestedNode = {
      guid: '0:0',
      type: 'FRAME',
      name: 'Parent',
      children: [
        {
          guid: '0:1',
          type: 'FRAME',
          name: 'Child 1',
          children: [
            {
              guid: '0:2',
              type: 'TEXT',
              name: 'Grandchild',
              children: [],
              properties: {
                textData: { characters: 'Hello World' },
                fontSize: 14,
                fontName: { family: 'Inter', style: 'Regular' },
              },
            },
          ],
          properties: {},
        },
      ],
      properties: {},
    }

    const prompt = generateFigmaPrompt(nestedNode, 'Test', 'Test', defaultPreset)

    expect(prompt).toContain('FRAME "Parent"')
    expect(prompt).toContain('→ FRAME "Child 1"')
    expect(prompt).toContain('→ TEXT "Grandchild"')
    expect(prompt).toContain('Content: "Hello World"')
    expect(prompt).toContain('Font: 14px Regular Inter')
  })
})

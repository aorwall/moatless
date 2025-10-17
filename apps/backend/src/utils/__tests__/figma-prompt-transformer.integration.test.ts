import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { GetFileResponse } from '@figma/rest-api-spec'
import {
  allExtractors,
  createFigmaClipboardParser,
  generateYamlFromExtractors,
  normalizeClipboardTree,
} from '@moatless/figma'
import type { Node } from '@moatless/types'
import { describe, expect, it } from 'vitest'

// Create temp directory for prompt outputs
const TEMP_DIR = '/tmp/figma-prompts-test'
mkdirSync(TEMP_DIR, { recursive: true })
console.log(`\n📁 Saving prompts to: ${TEMP_DIR}\n`)

describe('Figma YAML Generator Integration', () => {
  it('should transform figma-import-shadcn.json to YAML', () => {
    const fixturePath = join(
      __dirname,
      '../../services/__tests__/fixtures/figma-import-shadcn.json',
    )
    const importData = JSON.parse(readFileSync(fixturePath, 'utf-8'))

    // Wrap the node in a GetFileResponse structure
    const figmaResponse: GetFileResponse = {
      name: importData.fileName,
      lastModified: importData.createdAt,
      thumbnailUrl: '',
      version: '1.0',
      role: 'owner',
      editorType: 'figma',
      linkAccess: 'view',
      document: {
        id: '0:0',
        name: 'Document',
        type: 'DOCUMENT',
        scrollBehavior: 'SCROLLS',
        children: [importData.nodeMetadata as Node],
      },
      components: {},
      componentSets: {},
      schemaVersion: 0,
      styles: {},
    } as unknown as GetFileResponse

    const yaml = generateYamlFromExtractors(figmaResponse, {
      extractors: allExtractors,
    })

    // Save YAML to file for inspection
    const outputPath = join(TEMP_DIR, 'figma-import-shadcn.yaml')
    writeFileSync(outputPath, yaml, 'utf-8')
    console.log(`✅ Saved import YAML to: ${outputPath}`)

    // Verify YAML structure
    expect(yaml).toBeDefined()
    expect(yaml.length).toBeGreaterThan(500)

    // Verify YAML metadata
    expect(yaml).toContain('metadata:')
    expect(yaml).toContain('nodes:')

    // Verify it contains expected node types
    expect(yaml).toContain('type:')
    expect(yaml).toContain('name:')

    // Log sample of the YAML for debugging
    const lines = yaml.split('\n')
    console.log(`\n📝 Generated YAML (${lines.length} lines, ${yaml.length} chars)`)
    console.log('First 10 lines:')
    console.log(lines.slice(0, 10).join('\n'))
  })

  it('should transform figma-clipboard-shadcn.html to YAML', async () => {
    // Parse clipboard HTML
    const parser = createFigmaClipboardParser()

    const clipboardPath = join(__dirname, '../../services/__tests__/fixtures/figma-clipboard.html')
    const clipboardHtml = readFileSync(clipboardPath, 'utf-8')
    const parseResult = parser.parseClipboardHTML(clipboardHtml)

    // Normalize clipboard tree to REST API-compatible format
    const normalized = normalizeClipboardTree(parseResult.root, { contextRoot: parseResult.root })

    // Wrap the node in a GetFileResponse structure
    const figmaResponse: GetFileResponse = {
      name: normalized.name,
      lastModified: new Date().toISOString(),
      thumbnailUrl: '',
      version: '1.0',
      role: 'owner',
      editorType: 'figma',
      linkAccess: 'view',
      document: {
        id: '0:0',
        name: 'Document',
        type: 'DOCUMENT',
        scrollBehavior: 'SCROLLS',
        children: [normalized as Node],
      },
      components: {},
      componentSets: {},
      schemaVersion: 0,
      styles: {},
    } as unknown as GetFileResponse

    const yaml = generateYamlFromExtractors(figmaResponse, {
      extractors: allExtractors,
    })

    // Save YAML to file for inspection
    const outputPath = join(TEMP_DIR, 'figma-clipboard-shadcn.yaml')
    writeFileSync(outputPath, yaml, 'utf-8')
    console.log(`✅ Saved clipboard YAML to: ${outputPath}`)

    // Verify YAML structure
    expect(yaml).toBeDefined()
    expect(yaml.length).toBeGreaterThan(1000)
    expect(yaml).toContain('metadata:')
    expect(yaml).toContain('nodes:')
  })

  it('should handle node tree with layout properties', () => {
    const sampleNode = {
      id: '0:0',
      type: 'FRAME',
      name: 'Test Frame',
      children: [],
      absoluteBoundingBox: { x: 0, y: 0, width: 100, height: 200 },
      layoutMode: 'HORIZONTAL',
      itemSpacing: 16,
      fills: [
        {
          type: 'SOLID',
          color: { r: 1, g: 0, b: 0, a: 1 },
          visible: true,
        },
      ],
    } as Node

    const figmaResponse: GetFileResponse = {
      name: 'Test File',
      lastModified: new Date().toISOString(),
      thumbnailUrl: '',
      version: '1.0',
      role: 'owner',
      editorType: 'figma',
      linkAccess: 'view',
      document: {
        id: '0:0',
        name: 'Document',
        type: 'DOCUMENT',
        scrollBehavior: 'SCROLLS',
        children: [sampleNode],
      },
      components: {},
      componentSets: {},
      schemaVersion: 0,
      styles: {},
    } as unknown as GetFileResponse

    const yaml = generateYamlFromExtractors(figmaResponse, {
      extractors: allExtractors,
    })

    expect(yaml).toContain('type: FRAME')
    expect(yaml).toContain('name: Test Frame')
    expect(yaml).toContain('layoutMode: HORIZONTAL')
  })

  it('should handle nested children', () => {
    const nestedNode = {
      id: '0:0',
      type: 'FRAME',
      name: 'Parent',
      children: [
        {
          id: '0:1',
          type: 'FRAME',
          name: 'Child 1',
          children: [
            {
              id: '0:2',
              type: 'TEXT',
              name: 'Grandchild',
              characters: 'Hello World',
              style: {
                fontSize: 14,
                fontFamily: 'Inter',
                fontWeight: 400,
              },
            },
          ],
        },
      ],
    } as Node

    const figmaResponse: GetFileResponse = {
      name: 'Test',
      lastModified: new Date().toISOString(),
      thumbnailUrl: '',
      version: '1.0',
      role: 'owner',
      editorType: 'figma',
      linkAccess: 'view',
      document: {
        id: '0:0',
        name: 'Document',
        type: 'DOCUMENT',
        scrollBehavior: 'SCROLLS',
        children: [nestedNode],
      },
      components: {},
      componentSets: {},
      schemaVersion: 0,
      styles: {},
    } as unknown as GetFileResponse

    const yaml = generateYamlFromExtractors(figmaResponse, {
      extractors: allExtractors,
    })

    expect(yaml).toContain('type: FRAME')
    expect(yaml).toContain('name: Parent')
    expect(yaml).toContain('name: Child 1')
    expect(yaml).toContain('name: Grandchild')
    expect(yaml).toContain('Hello World')
  })
})

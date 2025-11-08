import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { createFigmaClipboardParser } from '@moatless/figma'
import { beforeEach, describe, expect, it } from 'vitest'

describe('FigmaClipboardParser Integration', () => {
  let parser: ReturnType<typeof createFigmaClipboardParser>
  let clipboardHtml: string

  beforeEach(() => {
    parser = createFigmaClipboardParser()
    // Load the real Figma clipboard HTML from fixtures
    const fixturePath = join(__dirname, 'fixtures', 'figma-clipboard.html')
    clipboardHtml = readFileSync(fixturePath, 'utf-8')
  })

  it('should parse real Figma clipboard data', () => {
    const result = parser.parseClipboardHTML(clipboardHtml)

    // Verify metadata extraction
    expect(result.metadata).toMatchObject({
      fileKey: 'Z6ij6B0g9en70YQ9sfLUQB',
      pasteID: 940559385,
      dataType: 'scene',
    })

    // Verify format type
    expect(result.formatType).toBe('fig-kiwie')

    // Verify version
    expect(result.version).toBeGreaterThan(0)

    // Verify node tree structure
    expect(result.root).toBeDefined()
    expect(result.root.guid).toBeDefined()
    expect(result.root.type).toBeDefined()
    expect(result.root.name).toBeDefined()

    // Verify we have a substantial number of nodes (real component should have many)
    console.log(`📊 Parsed ${result.nodeCount} nodes from Figma clipboard`)
    console.log(`📦 Root node: ${result.root.type} - "${result.root.name}"`)

    // The "User Action: Manual Override" component should have hundreds of nodes
    expect(result.nodeCount).toBeGreaterThan(100)
    expect(result.nodeCount).toBeLessThan(500) // Sanity check - not thousands
  })

  it('should build proper node tree hierarchy', () => {
    const result = parser.parseClipboardHTML(clipboardHtml)

    // Root should be defined with basic properties
    expect(result.root).toBeDefined()
    expect(result.root.guid).toBeDefined()
    expect(result.root.type).toBeDefined()
    expect(result.root.name).toBeDefined()

    // Children array should be defined and have content for a complex component
    expect(result.root.children).toBeDefined()
    expect(Array.isArray(result.root.children)).toBe(true)
    expect(result.root.children?.length).toBeGreaterThan(0)

    console.log(`🌳 Root has ${result.root.children?.length} direct children`)

    // Verify tree structure integrity by counting all nodes
    const countNodes = (node: typeof result.root): number => {
      let count = 1
      if (node.children && node.children.length > 0) {
        for (const child of node.children) {
          count += countNodes(child)
        }
      }
      return count
    }

    // Calculate tree depth
    const getDepth = (node: typeof result.root): number => {
      if (!node.children || node.children.length === 0) {
        return 1
      }
      return 1 + Math.max(...node.children.map(getDepth))
    }

    const actualCount = countNodes(result.root)
    const treeDepth = getDepth(result.root)

    console.log(`📏 Tree depth: ${treeDepth} levels`)
    console.log(`🔢 Actual nodes in tree: ${actualCount}`)

    // A real component should have significant depth
    expect(treeDepth).toBeGreaterThan(3) // At least 4 levels deep

    // actualCount should be <= nodeCount (nodeCount includes all changes, tree might be optimized)
    expect(actualCount).toBeGreaterThan(0)
    expect(actualCount).toBeLessThanOrEqual(result.nodeCount)
  })

  it('should parse node properties', () => {
    const result = parser.parseClipboardHTML(clipboardHtml)

    // Find first node with properties
    const findNodeWithProperties = (node: typeof result.root): typeof result.root | null => {
      if (node.properties && Object.keys(node.properties).length > 0) {
        return node
      }
      if (node.children) {
        for (const child of node.children) {
          const found = findNodeWithProperties(child)
          if (found) return found
        }
      }
      return null
    }

    const nodeWithProps = findNodeWithProperties(result.root)
    expect(nodeWithProps).toBeDefined()
    expect(nodeWithProps?.properties).toBeDefined()
    expect(typeof nodeWithProps?.properties).toBe('object')
  })

  it('should handle different node types', () => {
    const result = parser.parseClipboardHTML(clipboardHtml)

    // Collect all node types in the tree
    const nodeTypes = new Set<string>()
    const collectTypes = (node: typeof result.root) => {
      nodeTypes.add(node.type)
      if (node.children) {
        for (const child of node.children) {
          collectTypes(child)
        }
      }
    }

    collectTypes(result.root)

    // Should have at least one node type
    expect(nodeTypes.size).toBeGreaterThan(0)

    // Verify we captured valid Figma node types (not 'UNKNOWN')
    const types = Array.from(nodeTypes)
    expect(types.length).toBeGreaterThan(0)
    expect(types.every((type) => typeof type === 'string' && type.length > 0)).toBe(true)

    // Log node types for debugging
    console.log('Found node types:', types)
  })

  it('should throw error for invalid clipboard data', () => {
    const invalidHtml = '<div>Not Figma clipboard data</div>'

    expect(() => parser.parseClipboardHTML(invalidHtml)).toThrow()
  })

  it('should throw error for missing metadata', () => {
    const htmlWithoutMetadata = '<div><span data-buffer="base64data"></span></div>'

    expect(() => parser.parseClipboardHTML(htmlWithoutMetadata)).toThrow()
  })

  it('should throw error for missing buffer', () => {
    const htmlWithoutBuffer =
      '<div><span data-metadata="<!--(figmeta)eyJ0ZXN0IjoidmFsdWUifQ==(/figmeta)-->"></span></div>'

    expect(() => parser.parseClipboardHTML(htmlWithoutBuffer)).toThrow()
  })

  it('should save clipboard import and retrieve full node tree via storage', async () => {
    // This test verifies the complete unified import flow:
    // 1. Parse clipboard
    // 2. Create metadata with full node tree
    // 3. Save to storage
    // 4. Retrieve and verify full tree structure is preserved

    const { FigmaService } = await import('../../features/figma/services/figma.service.js')
    const { FigmaStorage } = await import('../../features/figma/storage/figma.storage.js')
    const { FileStorage } = await import('../../infrastructure/storage/file-storage.js')
    const { FileStorageService } = await import('../../features/file-storage/index.js')
    const { tmpdir } = await import('node:os')
    const { join } = await import('node:path')
    const { mkdtemp, rm, mkdir } = await import('node:fs/promises')

    // Create temporary directory for test data
    const tempDir = await mkdtemp(join(tmpdir(), 'figma-clipboard-test-'))

    // Set environment variable for upload service
    const originalUploadDir = process.env['UPLOAD_DIR']
    const uploadDir = join(tempDir, 'uploads')
    process.env['UPLOAD_DIR'] = uploadDir
    await mkdir(uploadDir, { recursive: true })

    try {
      // Create storage backend and figma storage adapter with temp directory
      const storageBackend = new FileStorage(tempDir)
      const figmaStorage = new FigmaStorage(storageBackend)
      const fileStorageService = new FileStorageService()

      // Create figma service with dependencies
      const figmaService = new FigmaService(figmaStorage, fileStorageService)

      // Import clipboard using the service
      const result = await figmaService.importClipboard(clipboardHtml)

      // Verify response format
      expect(result).toBeDefined()
      expect(typeof result.importId).toBe('string')
      expect(typeof result.fileId).toBe('string')
      expect(typeof result.nodeId).toBe('string')
      expect(typeof result.fileName).toBe('string')
      expect(typeof result.nodeName).toBe('string')
      expect(typeof result.createdAt).toBe('string')

      // Verify importId is a valid UUID
      expect(result.importId).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
      )

      // Retrieve saved metadata from storage
      const savedMetadata = await figmaStorage.getFigmaImport(result.importId)

      expect(savedMetadata).toBeDefined()
      expect(savedMetadata).not.toBeNull()

      if (!savedMetadata) {
        throw new Error('Metadata should be saved')
      }

      // Verify metadata fields
      expect(savedMetadata.importId).toBe(result.importId)
      expect(savedMetadata.fileId).toBe(result.fileId)
      expect(savedMetadata.nodeId).toBe(result.nodeId)
      expect(savedMetadata.fileName).toBe(result.fileName)
      expect(savedMetadata.nodeName).toBe(result.nodeName)

      // createdAt gets deserialized to Date object by storage fromJSON
      // Verify it matches the original string value
      const savedCreatedAt =
        typeof savedMetadata.createdAt === 'object' &&
        (savedMetadata.createdAt as unknown) instanceof Date
          ? (savedMetadata.createdAt as unknown as Date).toISOString()
          : (savedMetadata.createdAt as string)
      expect(savedCreatedAt).toBe(result.createdAt)
      expect(savedMetadata.url).toBeUndefined() // No URL for clipboard imports

      // Verify full node tree is preserved in nodeMetadata
      expect(savedMetadata.nodeMetadata).toBeDefined()
      const nodeTree = savedMetadata.nodeMetadata as {
        guid: string
        type: string
        name: string
        children?: unknown[]
        properties?: Record<string, unknown>
      }

      // Verify root node structure
      expect(nodeTree.guid).toBeDefined()
      expect(nodeTree.type).toBeDefined()
      expect(nodeTree.name).toBeDefined()
      expect(nodeTree.children).toBeDefined()
      expect(Array.isArray(nodeTree.children)).toBe(true)

      // Verify tree has substantial content
      expect(nodeTree.children?.length).toBeGreaterThan(0)

      // Count total nodes in tree
      const countNodes = (node: typeof nodeTree): number => {
        let count = 1
        if (node.children && Array.isArray(node.children)) {
          for (const child of node.children) {
            count += countNodes(child as typeof nodeTree)
          }
        }
        return count
      }

      const totalNodes = countNodes(nodeTree)
      console.log(`📦 Saved node tree has ${totalNodes} nodes`)

      // Should have many nodes (our test component has 289)
      expect(totalNodes).toBeGreaterThan(100)

      // Find a TEXT node to verify rich properties are preserved
      const findTextNode = (node: typeof nodeTree): typeof nodeTree | null => {
        if (node.type === 'TEXT' && node.properties) {
          return node
        }
        if (node.children && Array.isArray(node.children)) {
          for (const child of node.children) {
            const found = findTextNode(child as typeof nodeTree)
            if (found) return found
          }
        }
        return null
      }

      const textNode = findTextNode(nodeTree)
      expect(textNode).toBeDefined()
      expect(textNode?.properties).toBeDefined()

      // Verify rich Figma properties are preserved
      const props = textNode?.properties
      if (props) {
        console.log('📝 TEXT node properties keys:', Object.keys(props))
        // Properties should include Figma-specific fields
        expect(Object.keys(props).length).toBeGreaterThan(5)
      }
    } finally {
      // Restore environment variable
      if (originalUploadDir) {
        process.env['UPLOAD_DIR'] = originalUploadDir
      } else {
        process.env['UPLOAD_DIR'] = undefined
      }

      // Cleanup
      await rm(tempDir, { recursive: true, force: true })
    }
  })
})

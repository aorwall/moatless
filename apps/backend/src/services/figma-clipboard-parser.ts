import * as kiwi from 'kiwi-schema'
import * as pako from 'pako'
import { ZSTDDecoder } from 'zstddec'
import { logger } from '../utils/logger.js'

/**
 * Metadata extracted from Figma clipboard
 */
export interface FigmaClipboardMetadata {
  fileKey: string
  pasteID: number
  dataType: string
}

/**
 * Figma GUID structure used in clipboard format
 */
interface FigmaGuid {
  sessionID: number
  localID: number
}

/**
 * Parent index structure in Figma clipboard
 */
interface ParentIndex {
  guid: FigmaGuid
  position: string
}

/**
 * Raw node change from Kiwi decoding
 */
interface NodeChange {
  guid?: FigmaGuid | string
  type?: string
  name?: string
  parentIndex?: ParentIndex
  childIndex?: number
  [key: string]: unknown
}

/**
 * Raw blob data from Kiwi decoding
 */
interface BlobData {
  key?: string
  bytes?: Uint8Array
  [key: string]: unknown
}

/**
 * Parsed Figma node tree
 */
export interface FigmaNode {
  guid: string
  type: string
  name: string
  children?: FigmaNode[]
  properties?: Record<string, unknown>
  blobs?: Record<string, unknown>
}

/**
 * Result of parsing Figma clipboard data
 */
export interface FigmaClipboardParseResult {
  metadata: FigmaClipboardMetadata
  version: number
  root: FigmaNode
  nodeCount: number
  formatType: 'fig-kiwie' | 'fig-jam.' | 'fig-deck'
}

/**
 * Service for parsing Figma clipboard data
 * Decodes the binary fig-kiwie format from clipboard HTML
 */
export class FigmaClipboardParser {
  /**
   * Parse Figma clipboard HTML content
   * @param htmlContent - Raw HTML clipboard content from Figma
   * @returns Parsed Figma design data
   */
  parseClipboardHTML(htmlContent: string): FigmaClipboardParseResult {
    logger.debug('Parsing Figma clipboard HTML')

    // Extract metadata and binary buffer
    const metadata = this.extractMetadata(htmlContent)
    const binaryBuffer = this.extractBinaryBuffer(htmlContent)

    if (!(metadata && binaryBuffer)) {
      throw new Error('Invalid Figma clipboard format: missing metadata or binary data')
    }

    // Decode the binary fig-kiwie format
    const parseResult = this.decodeFigmaBuffer(binaryBuffer)

    return {
      metadata,
      ...parseResult,
    }
  }

  /**
   * Decode HTML entities in a string
   */
  private decodeHtmlEntities(text: string): string {
    return text
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&amp;/g, '&')
      .replace(/&quot;/g, '"')
      .replace(/&#39;/g, "'")
  }

  /**
   * Extract metadata from clipboard HTML
   * Parses the data-metadata span containing base64-encoded JSON
   */
  private extractMetadata(html: string): FigmaClipboardMetadata | null {
    try {
      // Decode HTML entities first
      const decodedHtml = this.decodeHtmlEntities(html)

      // Find data-metadata span with pattern: <!--(figmeta)BASE64(/figmeta)-->
      const metadataMatch = decodedHtml.match(
        /data-metadata="<!--\(figmeta\)([^)]+)\(\/figmeta\)-->"/,
      )
      if (!metadataMatch?.[1]) {
        logger.warn('No metadata found in clipboard HTML')
        return null
      }

      // Decode base64 and parse JSON
      const base64Metadata = metadataMatch[1]
      const jsonString = Buffer.from(base64Metadata, 'base64').toString('utf-8')
      const metadata = JSON.parse(jsonString) as FigmaClipboardMetadata

      logger.debug('Extracted Figma clipboard metadata', { metadata })
      return metadata
    } catch (error) {
      logger.error('Failed to extract clipboard metadata', error as Error)
      return null
    }
  }

  /**
   * Extract binary buffer from clipboard HTML
   * Parses the data-buffer span containing base64-encoded binary data
   */
  private extractBinaryBuffer(html: string): Uint8Array | null {
    try {
      // Decode HTML entities first
      const decodedHtml = this.decodeHtmlEntities(html)

      // Find data-buffer span with pattern: <!--(figma)BASE64(/figma)-->
      const bufferMatch = decodedHtml.match(/data-buffer="<!--\(figma\)([^)]+)\(\/figma\)-->"/)
      if (!bufferMatch?.[1]) {
        logger.warn('No binary buffer found in clipboard HTML')
        return null
      }

      // Decode base64 to binary
      const base64Buffer = bufferMatch[1]
      const binaryData = Buffer.from(base64Buffer, 'base64')
      const uint8Array = new Uint8Array(binaryData)

      logger.debug('Extracted binary buffer', { size: uint8Array.length })
      return uint8Array
    } catch (error) {
      logger.error('Failed to extract binary buffer', error as Error)
      return null
    }
  }

  /**
   * Decode Figma binary buffer (fig-kiwie format)
   * Parses the binary structure: [header][version][schema_chunk][data_chunk]
   */
  private decodeFigmaBuffer(buffer: Uint8Array): {
    version: number
    root: FigmaNode
    nodeCount: number
    formatType: 'fig-kiwie' | 'fig-jam.' | 'fig-deck'
  } {
    let offset = 0

    // Read header (8 bytes) - should be "fig-kiwi", "fig-jam.", or "fig-deck"
    const headerBytes = buffer.slice(offset, offset + 8)
    const header = new TextDecoder().decode(headerBytes)
    offset += 8

    logger.debug('Figma buffer header', { header })

    let formatType: 'fig-kiwie' | 'fig-jam.' | 'fig-deck'
    if (header.startsWith('fig-kiwi')) {
      formatType = 'fig-kiwie'
    } else if (header.startsWith('fig-jam.')) {
      formatType = 'fig-jam.'
    } else if (header.startsWith('fig-deck')) {
      formatType = 'fig-deck'
    } else {
      throw new Error(`Unknown Figma format: ${header}`)
    }

    // Read version (4 bytes, Uint32)
    const version = new DataView(buffer.buffer, buffer.byteOffset + offset, 4).getUint32(0, true)
    offset += 4
    logger.debug('Figma buffer version', { version })

    // Read schema chunk
    const schemaChunkLength = new DataView(buffer.buffer, buffer.byteOffset + offset, 4).getUint32(
      0,
      true,
    )
    offset += 4
    const schemaChunk = buffer.slice(offset, offset + schemaChunkLength)
    offset += schemaChunkLength

    // Read data chunk
    const dataChunkLength = new DataView(buffer.buffer, buffer.byteOffset + offset, 4).getUint32(
      0,
      true,
    )
    offset += 4
    const dataChunk = buffer.slice(offset, offset + dataChunkLength)

    logger.debug('Binary structure', {
      version,
      schemaChunkLength,
      dataChunkLength,
      totalSize: buffer.length,
    })

    // Decompress chunks
    const decompressedSchema = this.uncompressChunk(schemaChunk)
    const decompressedData = this.uncompressChunk(dataChunk)

    // Decode schema using kiwi
    const binarySchema = kiwi.decodeBinarySchema(decompressedSchema)
    const schema = kiwi.compileSchema(binarySchema)

    // Decode data using schema
    const message = schema.decodeMessage(decompressedData)

    logger.debug('Decoded message', {
      keys: Object.keys(message),
      nodeChangesCount: message.nodeChanges?.length ?? 0,
      blobsCount: message.blobs?.length ?? 0,
    })

    // Build node tree from nodeChanges
    const root = this.buildNodeTree(message.nodeChanges ?? [], message.blobs ?? [])

    return {
      version,
      root,
      nodeCount: message.nodeChanges?.length ?? 0,
      formatType,
    }
  }

  /**
   * Uncompress a chunk using DEFLATE or ZSTD
   * Tries DEFLATE first, falls back to ZSTD if that fails
   */
  private uncompressChunk(chunk: Uint8Array): Uint8Array {
    try {
      // Try DEFLATE decompression first (most common)
      const decompressed = pako.inflateRaw(chunk)
      logger.debug('Decompressed chunk with DEFLATE', {
        originalSize: chunk.length,
        decompressedSize: decompressed.length,
      })
      return decompressed
    } catch (deflateError) {
      try {
        // Fall back to ZSTD decompression
        const decoder = new ZSTDDecoder()
        decoder.init().then(() => {
          logger.debug('Initialized ZSTD decoder')
        })

        const decompressed = decoder.decode(chunk)
        logger.debug('Decompressed chunk with ZSTD', {
          originalSize: chunk.length,
          decompressedSize: decompressed.length,
        })
        return decompressed
      } catch (zstdError) {
        // If both fail, the chunk might not be compressed
        logger.warn('Could not decompress chunk, using raw data', {
          deflateError: deflateError instanceof Error ? deflateError.message : String(deflateError),
          zstdError: zstdError instanceof Error ? zstdError.message : String(zstdError),
        })
        return chunk
      }
    }
  }

  /**
   * Convert FigmaGuid to string key for node map
   */
  private guidToString(guid: FigmaGuid | string | undefined): string {
    if (!guid) {
      return ''
    }
    if (typeof guid === 'string') {
      return guid
    }
    return `${guid.sessionID}:${guid.localID}`
  }

  /**
   * Build node tree from flat nodeChanges array
   * Converts flat list to hierarchical tree structure
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex tree-building algorithm with multiple necessary steps
  private buildNodeTree(nodeChanges: NodeChange[], blobs: BlobData[]): FigmaNode {
    logger.info('Building node tree', { nodeCount: nodeChanges.length })

    // Create a map of guid -> node for quick lookup
    const nodeMap = new Map<string, FigmaNode>()

    // First pass: create all nodes
    for (const change of nodeChanges) {
      const guidKey = this.guidToString(change.guid)
      const node: FigmaNode = {
        guid: guidKey,
        type: change.type || 'UNKNOWN',
        name: change.name || 'Unnamed',
        children: [],
        properties: change,
      }

      nodeMap.set(guidKey, node)
    }

    // Second pass: attach blobs to nodes
    for (const blob of blobs) {
      if (blob.key && blob.bytes) {
        const parsedBlob = this.parseBlob(blob.key, blob.bytes)
        if (parsedBlob) {
          // Find node that references this blob
          for (const node of nodeMap.values()) {
            if (node.properties && this.nodeReferencesBlob(node.properties, blob.key)) {
              if (!node.blobs) {
                node.blobs = {}
              }
              node.blobs[blob.key] = parsedBlob
            }
          }
        }
      }
    }

    // Third pass: build parent-child relationships using parentIndex
    let root: FigmaNode | null = null
    let nodesWithParent = 0
    let nodesWithoutParent = 0

    for (const change of nodeChanges) {
      const guidKey = this.guidToString(change.guid)
      const node = nodeMap.get(guidKey)
      if (!node) {
        continue
      }

      // parentIndex is an object: {guid: {sessionID, localID}, position}
      const parentIndex = change.parentIndex

      if (parentIndex?.guid) {
        // This node has a parent - find parent by matching guid
        const parentGuidKey = this.guidToString(parentIndex.guid)
        const foundParent = nodeMap.get(parentGuidKey) ?? null

        if (foundParent?.children) {
          nodesWithParent++
          foundParent.children.push(node)
        }
      } else {
        // This is a root node (no parent index)
        nodesWithoutParent++
        if (!root) {
          root = node
        }
      }
    }

    logger.info('Parent-child linking complete', {
      nodesWithParent,
      nodesWithoutParent,
      totalNodes: nodeChanges.length,
    })

    // Sort children by childIndex if available
    for (const node of nodeMap.values()) {
      if (node.children && node.children.length > 0) {
        node.children.sort((a, b) => {
          const propsA = a.properties as NodeChange | undefined
          const propsB = b.properties as NodeChange | undefined
          const indexA = propsA?.childIndex ?? 0
          const indexB = propsB?.childIndex ?? 0
          return indexA - indexB
        })
      }
    }

    if (!root) {
      // If no root found, create a synthetic root
      logger.warn('No root node found, creating synthetic root')
      root = {
        guid: 'synthetic-root',
        type: 'DOCUMENT',
        name: 'Document',
        children: Array.from(nodeMap.values()),
      }
    }

    return root
  }

  /**
   * Check if a node references a specific blob key
   */
  private nodeReferencesBlob(properties: Record<string, unknown>, blobKey: string): boolean {
    // Check if any property references this blob key
    const propsStr = JSON.stringify(properties)
    return propsStr.includes(blobKey)
  }

  /**
   * Parse blob data (commands, vectorNetwork, etc.)
   */
  private parseBlob(key: string, bytes: Uint8Array): unknown {
    try {
      // Blobs can contain different types of data:
      // - commands: SVG path commands
      // - vectorNetwork: Vector network data
      // - image: Image data

      if (key.includes('commands')) {
        // Parse as SVG path commands
        return this.parseCommands(bytes)
      }

      if (key.includes('vectorNetwork')) {
        // Parse as vector network
        return this.parseVectorNetwork(bytes)
      }

      // For other blobs, return as base64 for now
      return {
        type: 'binary',
        size: bytes.length,
        base64: Buffer.from(bytes).toString('base64'),
      }
    } catch (error) {
      logger.warn('Failed to parse blob', {
        key,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }

  /**
   * Parse SVG path commands from binary data
   */
  private parseCommands(bytes: Uint8Array): unknown {
    // Commands are typically stored as a series of floats
    // This is a simplified parser - real implementation would need proper parsing
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    const commands: number[] = []

    for (let i = 0; i < bytes.length; i += 4) {
      if (i + 4 <= bytes.length) {
        commands.push(view.getFloat32(i, true))
      }
    }

    return { type: 'commands', data: commands }
  }

  /**
   * Parse vector network from binary data
   */
  private parseVectorNetwork(bytes: Uint8Array): unknown {
    // Vector network contains vertices, segments, and regions
    // This is a simplified parser
    return {
      type: 'vectorNetwork',
      size: bytes.length,
      // Full parsing would require the kiwi schema for vector networks
    }
  }
}

/**
 * Create a new FigmaClipboardParser instance
 */
export function createFigmaClipboardParser(): FigmaClipboardParser {
  return new FigmaClipboardParser()
}

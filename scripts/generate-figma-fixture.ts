/**
 * Generate expected output fixture for Rust parity testing.
 *
 * Runs the TypeScript Figma clipboard parser on the test fixture HTML
 * and serializes the result to JSON for comparison with the Rust parser.
 *
 * Usage: npx tsx scripts/generate-figma-fixture.ts
 */

import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createFigmaClipboardParser } from '@moatless/figma'

const fixturePath = join(__dirname, '../packages/figma/src/__tests__/fixtures/figma-clipboard.html')
const outputPath = join(__dirname, '../crates/figma-clipboard/tests/fixtures/expected-output.json')

const html = readFileSync(fixturePath, 'utf-8')
const parser = createFigmaClipboardParser()
const result = parser.parseClipboardHTML(html)

// Extract the key fields we want to compare
const output = {
  metadata: result.metadata,
  version: result.version,
  formatType: result.formatType,
  nodeCount: result.nodeCount,
  root: serializeNode(result.root),
}

function serializeNode(node: typeof result.root): unknown {
  return {
    guid: node.guid,
    type: node.type,
    name: node.name,
    childCount: node.children?.length ?? 0,
    children: node.children?.map(serializeNode) ?? [],
  }
}

writeFileSync(outputPath, JSON.stringify(output, null, 2))
console.log(`Generated fixture: ${outputPath}`)
console.log(`  metadata.fileKey: ${result.metadata.fileKey}`)
console.log(`  metadata.pasteID: ${result.metadata.pasteID}`)
console.log(`  nodeCount: ${result.nodeCount}`)
console.log(`  formatType: ${result.formatType}`)
console.log(`  root.type: ${result.root.type}`)
console.log(`  root.name: ${result.root.name}`)
console.log(`  root.children: ${result.root.children?.length ?? 0}`)

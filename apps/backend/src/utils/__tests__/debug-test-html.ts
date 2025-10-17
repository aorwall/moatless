import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { GetFileResponse } from '@figma/rest-api-spec'
import {
  allExtractors,
  createFigmaClipboardParser,
  generateYamlFromExtractors,
  normalizeClipboardTree,
} from '@moatless/figma'
import type { Node } from '@moatless/types'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)

// Parse test.html
const testHtmlPath = join(__dirname, '../../services/__tests__/fixtures/test.html')
const clipboardHtml = readFileSync(testHtmlPath, 'utf-8')

console.log('📦 Parsing test.html clipboard data...\n')

const parser = createFigmaClipboardParser()
const parseResult = parser.parseClipboardHTML(clipboardHtml)

console.log('✅ Parsed clipboard data:')
console.log(`- Format: ${parseResult.formatType}`)
console.log(`- Version: ${parseResult.version}`)
console.log(`- Node count: ${parseResult.nodeCount}`)
console.log(`- Root node: ${parseResult.root.name} (${parseResult.root.type})`)

// Function to recursively search for text overrides
function findTextOverrides(node: any, path = '', depth = 0): void {
  const currentPath = path ? `${path} > ${node.name}` : node.name
  const indent = '  '.repeat(depth)

  // Check for symbolOverrides
  if (node.properties?.symbolOverrides) {
    console.log(`\n${indent}🔍 Found symbolOverrides at: ${currentPath}`)
    const overrides = node.properties.symbolOverrides
    console.log(`${indent}   Count: ${overrides.length}`)

    // Check for text overrides
    const textOverrides = overrides.filter((o: any) => o.textData?.characters)
    if (textOverrides.length > 0) {
      console.log(`${indent}   📝 Text overrides (${textOverrides.length}):`)
      textOverrides.forEach((o: any, i: number) => {
        console.log(`${indent}      ${i + 1}. "${o.textData.characters}"`)
      })
    } else {
      console.log(`${indent}   ⚠️  No text overrides found in symbolOverrides`)
      // Show what kinds of overrides exist
      const overrideTypes = new Set(overrides.map((o: any) => Object.keys(o).join(', ')))
      console.log(`${indent}   Available override types: ${Array.from(overrideTypes).join(' | ')}`)
    }
  }

  // Check for componentPropAssignments
  if (node.properties?.componentPropAssignments) {
    console.log(`\n${indent}🔍 Found componentPropAssignments at: ${currentPath}`)
    const assignments = node.properties.componentPropAssignments
    console.log(`${indent}   Count: ${assignments.length}`)

    // Look for text-related assignments
    const textAssignments = assignments.filter((a: any) => {
      const value = a.value || {}
      return value.textValue || value.textDataValue
    })

    if (textAssignments.length > 0) {
      console.log(`${indent}   📝 Text assignments (${textAssignments.length}):`)
      textAssignments.forEach((a: any, i: number) => {
        const text = a.value?.textValue?.characters || a.value?.textDataValue?.characters
        console.log(`${indent}      ${i + 1}. Property: ${a.propName}, Text: "${text}"`)
      })
    } else {
      console.log(`${indent}   ⚠️  No text assignments`)
      // Show what kinds of assignments exist
      const assignmentTypes = new Set(
        assignments.map((a: any) => `${a.propName}: ${Object.keys(a.value || {}).join(', ')}`),
      )
      console.log(`${indent}   Available assignments:`)
      Array.from(assignmentTypes).forEach((t) => {
        console.log(`${indent}      - ${t}`)
      })
    }
  }

  // Check for textData at node level
  if (node.properties?.textData?.characters) {
    console.log(`\n${indent}📝 Found textData.characters at: ${currentPath}`)
    console.log(`${indent}   Text: "${node.properties.textData.characters}"`)
  }

  // Recurse into children
  if (node.children && node.children.length > 0) {
    for (const child of node.children) {
      findTextOverrides(child, currentPath, depth + 1)
    }
  }
}

console.log('\n=== Searching for text overrides in parsed tree ===')
findTextOverrides(parseResult.root)

// Now normalize the tree
console.log('\n\n=== Normalizing clipboard tree ===')
const normalized = normalizeClipboardTree(parseResult.root, { contextRoot: parseResult.root })

// Save normalized structure for inspection
const normalizedPath = '/tmp/test-html-normalized.json'
writeFileSync(normalizedPath, JSON.stringify(normalized, null, 2), 'utf-8')
console.log(`\n💾 Saved normalized tree to: ${normalizedPath}`)

// Function to find text nodes in normalized tree
function findTextNodes(node: any, path = '', depth = 0): void {
  const currentPath = path ? `${path} > ${node.name}` : node.name
  const indent = '  '.repeat(depth)

  if (node.type === 'TEXT') {
    console.log(`\n${indent}📝 TEXT node: ${currentPath}`)
    console.log(`${indent}   ID: ${node.id}`)
    console.log(`${indent}   characters: "${node.characters || '(none)'}"`)

    // Check for override marker
    if (node.__normalizedTextOverride !== undefined) {
      console.log(`${indent}   ✅ Has __normalizedTextOverride: "${node.__normalizedTextOverride}"`)
    } else {
      console.log(`${indent}   ❌ No __normalizedTextOverride`)
    }
  }

  // Recurse into children
  if (node.children && node.children.length > 0) {
    for (const child of node.children) {
      findTextNodes(child, currentPath, depth + 1)
    }
  }
}

console.log('\n=== Searching for TEXT nodes in normalized tree ===')
findTextNodes(normalized)

// Generate YAML
console.log('\n\n=== Generating YAML ===')
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

const yamlPath = '/tmp/test-html-generated.yaml'
writeFileSync(yamlPath, yaml, 'utf-8')
console.log(`\n💾 Saved generated YAML to: ${yamlPath}`)

// Show first 50 lines of YAML
const yamlLines = yaml.split('\n')
console.log(`\n📄 Generated YAML (${yamlLines.length} lines):`)
console.log(yamlLines.slice(0, 50).join('\n'))
console.log('\n...')

console.log('\n\n=== Summary ===')
console.log('✅ Debug script complete')
console.log('📂 Check these files for details:')
console.log(`   - ${normalizedPath}`)
console.log(`   - ${yamlPath}`)

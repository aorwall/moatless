import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createFigmaClipboardParser } from '@moatless/figma'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)

// Function to analyze clipboard structure
function analyzeClipboard(htmlPath: string, name: string) {
  console.log(`\n${'='.repeat(80)}`)
  console.log(`📦 Analyzing: ${name}`)
  console.log('='.repeat(80))

  const clipboardHtml = readFileSync(htmlPath, 'utf-8')
  const parser = createFigmaClipboardParser()
  const parseResult = parser.parseClipboardHTML(clipboardHtml)

  console.log('\n✅ Parsed data:')
  console.log(`   - Format: ${parseResult.formatType}`)
  console.log(`   - Version: ${parseResult.version}`)
  console.log(`   - Node count: ${parseResult.nodeCount}`)
  console.log(`   - Root: ${parseResult.root.name} (${parseResult.root.type})`)

  // Count text overrides
  let symbolOverridesCount = 0
  let symbolOverridesWithText = 0
  let componentPropAssignmentsCount = 0
  let componentPropAssignmentsWithText = 0
  let textDataCount = 0

  function scanNode(node: any): void {
    // Check for symbolOverrides
    if (node.properties?.symbolOverrides) {
      symbolOverridesCount++
      const overrides = node.properties.symbolOverrides
      const textOverrides = overrides.filter((o: any) => o.textData?.characters)
      if (textOverrides.length > 0) {
        symbolOverridesWithText += textOverrides.length
        console.log(`\n   📝 Symbol overrides with text at: ${node.name}`)
        textOverrides.slice(0, 3).forEach((o: any) => {
          console.log(`      - "${o.textData.characters}"`)
        })
        if (textOverrides.length > 3) {
          console.log(`      ... and ${textOverrides.length - 3} more`)
        }
      }
    }

    // Check for componentPropAssignments
    if (node.properties?.componentPropAssignments) {
      componentPropAssignmentsCount++
      const assignments = node.properties.componentPropAssignments
      const textAssignments = assignments.filter((a: any) => {
        const value = a.value || {}
        return value.textValue || value.textDataValue
      })
      if (textAssignments.length > 0) {
        componentPropAssignmentsWithText += textAssignments.length
        console.log(`\n   📝 Prop assignments with text at: ${node.name}`)
        textAssignments.slice(0, 3).forEach((a: any) => {
          const text = a.value?.textValue?.characters || a.value?.textDataValue?.characters
          console.log(`      - Property: ${a.propName}, Text: "${text}"`)
        })
        if (textAssignments.length > 3) {
          console.log(`      ... and ${textAssignments.length - 3} more`)
        }
      }
    }

    // Check for textData
    if (node.properties?.textData?.characters) {
      textDataCount++
    }

    // Recurse
    if (node.children && node.children.length > 0) {
      for (const child of node.children) {
        scanNode(child)
      }
    }
  }

  scanNode(parseResult.root)

  console.log('\n📊 Summary:')
  console.log(`   - Nodes with symbolOverrides: ${symbolOverridesCount}`)
  console.log(`   - Symbol overrides with text: ${symbolOverridesWithText}`)
  console.log(`   - Nodes with componentPropAssignments: ${componentPropAssignmentsCount}`)
  console.log(`   - Prop assignments with text: ${componentPropAssignmentsWithText}`)
  console.log(`   - Nodes with textData.characters: ${textDataCount}`)

  return {
    symbolOverridesWithText,
    componentPropAssignmentsWithText,
    textDataCount,
  }
}

// Analyze both files
const testHtmlPath = join(__dirname, '../../services/__tests__/fixtures/test.html')
const workingClipboardPath = join(
  __dirname,
  '../../services/__tests__/fixtures/figma-clipboard.html',
)

const testResults = analyzeClipboard(testHtmlPath, 'test.html (BROKEN)')
const workingResults = analyzeClipboard(workingClipboardPath, 'figma-clipboard.html (WORKING)')

console.log(`\n\n${'='.repeat(80)}`)
console.log('🔍 COMPARISON')
console.log('='.repeat(80))

console.log('\n📊 Side-by-side comparison:')
console.log('\n                                    test.html    figma-clipboard.html')
console.log('                                    ---------    --------------------')
console.log(
  `Symbol overrides with text:         ${String(testResults.symbolOverridesWithText).padStart(9)}    ${String(workingResults.symbolOverridesWithText).padStart(20)}`,
)
console.log(
  `Prop assignments with text:         ${String(testResults.componentPropAssignmentsWithText).padStart(9)}    ${String(workingResults.componentPropAssignmentsWithText).padStart(20)}`,
)
console.log(
  `Nodes with textData.characters:     ${String(testResults.textDataCount).padStart(9)}    ${String(workingResults.textDataCount).padStart(20)}`,
)

console.log('\n\n🎯 CONCLUSION:')
if (testResults.symbolOverridesWithText === 0 && workingResults.symbolOverridesWithText > 0) {
  console.log(
    '❌ test.html is missing symbolOverrides with text data that figma-clipboard.html has.',
  )
  console.log(
    '   This means the clipboard data was copied from Figma WITHOUT instance text overrides.',
  )
  console.log('   The normalizer cannot extract text that is not in the clipboard data.')
} else if (
  testResults.componentPropAssignmentsWithText === 0 &&
  workingResults.componentPropAssignmentsWithText > 0
) {
  console.log(
    '❌ test.html is missing componentPropAssignments with text that figma-clipboard.html has.',
  )
  console.log(
    '   This means the clipboard data was copied from Figma WITHOUT component property text values.',
  )
} else if (
  testResults.symbolOverridesWithText === 0 &&
  testResults.componentPropAssignmentsWithText === 0
) {
  console.log('❌ test.html has NO text overrides at all.')
  console.log('   The clipboard data contains only component definition placeholder text.')
  console.log('   To fix this, copy the Figma design with instances that have custom text values.')
} else {
  console.log('✅ test.html has text overrides - the issue must be in the normalization pipeline.')
}

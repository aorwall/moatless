/**
 * Generate expected prompt output fixture for Rust parity testing.
 *
 * Runs the TypeScript Figma prompt pipeline (parse → prompt) on the test
 * fixture HTML and saves the resulting prompt text for comparison with
 * the Rust pipeline (parse → normalize → prompt).
 *
 * Usage: npx tsx scripts/generate-figma-prompt-fixture.ts
 */

import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createFigmaClipboardParser } from '../packages/figma/src/clipboard/parser.js'
import { generateFigmaPrompt } from '../packages/figma/src/prompt/generator.js'
import { clipboardPreset } from '../packages/figma/src/config/presets.js'

const fixturePath = join(__dirname, '../packages/figma/src/__tests__/fixtures/figma-clipboard.html')
const outputPath = join(__dirname, '../crates/figma-clipboard/tests/fixtures/expected-prompt.txt')

const html = readFileSync(fixturePath, 'utf-8')
const parser = createFigmaClipboardParser()
const result = parser.parseClipboardHTML(html)

const prompt = generateFigmaPrompt(
  result.root,
  'Test File',
  'Test Component',
  clipboardPreset,
)

writeFileSync(outputPath, prompt)
console.log(`Generated prompt fixture: ${outputPath}`)
console.log(`  Prompt length: ${prompt.length} chars`)
console.log(`  Lines: ${prompt.split('\n').length}`)
console.log(`  First 200 chars:`)
console.log(prompt.slice(0, 200))

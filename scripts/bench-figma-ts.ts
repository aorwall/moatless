/**
 * Benchmark the TypeScript Figma clipboard parser for comparison with Rust.
 *
 * Usage: bun run scripts/bench-figma-ts.ts
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { createFigmaClipboardParser } from '@moatless/figma'

const fixturePath = join(__dirname, '../packages/figma/src/__tests__/fixtures/figma-clipboard.html')
const html = readFileSync(fixturePath, 'utf-8')
const parser = createFigmaClipboardParser()

// Warmup
for (let i = 0; i < 5; i++) {
  parser.parseClipboardHTML(html)
}

// Benchmark
const iterations = 50
const times: number[] = []

for (let i = 0; i < iterations; i++) {
  const start = performance.now()
  parser.parseClipboardHTML(html)
  const elapsed = performance.now() - start
  times.push(elapsed)
}

times.sort((a, b) => a - b)

const mean = times.reduce((a, b) => a + b, 0) / times.length
const median = times[Math.floor(times.length / 2)]
const p5 = times[Math.floor(times.length * 0.05)]
const p95 = times[Math.floor(times.length * 0.95)]
const min = times[0]
const max = times[times.length - 1]

console.log(`TypeScript Figma clipboard parser benchmark (${iterations} iterations):`)
console.log(`  Mean:   ${mean.toFixed(3)} ms`)
console.log(`  Median: ${median.toFixed(3)} ms`)
console.log(`  Min:    ${min.toFixed(3)} ms`)
console.log(`  Max:    ${max.toFixed(3)} ms`)
console.log(`  P5:     ${p5.toFixed(3)} ms`)
console.log(`  P95:    ${p95.toFixed(3)} ms`)
console.log(`  Throughput: ${((html.length / 1024 / 1024) / (mean / 1000)).toFixed(2)} MiB/s`)

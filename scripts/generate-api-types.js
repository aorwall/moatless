#!/usr/bin/env node
import fs from 'fs/promises'
import path from 'path'
import { fileURLToPath } from 'url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

const API_URL = process.env.VITE_API_URL || 'http://localhost:8000'
const OPENAPI_URL = `${API_URL}/openapi.json`
const OUTPUT_PATH = path.join(__dirname, '../src/lib/api/openapi/spec.json')

async function fetchOpenAPISpec() {
  try {
    console.log(`Fetching OpenAPI spec from ${OPENAPI_URL}...`)

    const response = await fetch(OPENAPI_URL)
    if (!response.ok) {
      throw new Error(`Failed to fetch OpenAPI spec: ${response.statusText}`)
    }

    const spec = await response.json()

    // Ensure directory exists
    const dir = path.dirname(OUTPUT_PATH)
    await fs.mkdir(dir, { recursive: true })

    // Write spec to file
    await fs.writeFile(OUTPUT_PATH, JSON.stringify(spec, null, 2))

    console.log(`OpenAPI spec saved to ${OUTPUT_PATH}`)
    console.log('You can now use this spec to generate TypeScript types')

    // Instructions for next steps
    console.log('\nNext steps:')
    console.log('1. Install openapi-typescript: pnpm add -D openapi-typescript')
    console.log(
      '2. Generate types: npx openapi-typescript src/lib/api/openapi/spec.json -o src/lib/api/openapi/types.ts',
    )
  } catch (error) {
    console.error('Error fetching OpenAPI spec:', error)
    process.exit(1)
  }
}

fetchOpenAPISpec()

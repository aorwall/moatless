import { createRequire } from 'module'
import { fileURLToPath } from 'url'
import { dirname } from 'path'

// Only define if not already defined
if (typeof globalThis.__filename === 'undefined') {
  globalThis.__filename = fileURLToPath(import.meta.url)
  globalThis.__dirname = dirname(globalThis.__filename)
}

if (typeof globalThis.require === 'undefined') {
  globalThis.require = createRequire(import.meta.url)
}

// Export for injection
export const __filename = globalThis.__filename
export const __dirname = globalThis.__dirname
export const require = globalThis.require
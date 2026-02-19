/**
 * Load environment variables from docker-compose/.env.docker-compose.
 *
 * Must be imported before any other helpers to ensure env vars are available.
 * Does not override variables already set in the shell.
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const ENV_PATH = path.resolve(__dirname, '../../../docker-compose/.env.docker-compose')

if (fs.existsSync(ENV_PATH)) {
  for (const line of fs.readFileSync(ENV_PATH, 'utf-8').split('\n')) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#')) {
      continue
    }
    const eqIdx = trimmed.indexOf('=')
    if (eqIdx === -1) {
      continue
    }
    const key = trimmed.slice(0, eqIdx)
    const value = trimmed.slice(eqIdx + 1)
    if (!process.env[key]) {
      process.env[key] = value
    }
  }
}

/** Resolved backend URL from BACKEND_URL or BACKEND_PORT. */
export const BACKEND_URL =
  process.env['BACKEND_URL'] ||
  `http://localhost:${process.env['BACKEND_PORT'] || '8080'}`

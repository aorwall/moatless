/**
 * Where the UI suite points and how it signs in. Read once, by the config and
 * by global setup, and failed loudly when missing: a run against the wrong
 * deployment must not look like a run that passed.
 */

import { resolve } from 'node:path'

export interface UiEnv {
  /** Origin serving both the T3 client and the Moatless API, e.g. `https://moatless.example.com`. */
  baseUrl: string
  /** An `mvk_` key: exchanged for the browser's session and sent as a bearer by `Backend`. */
  apiKey: string
  /** Project (Moatless workspace) name as the T3 command palette lists it. */
  project: string
}

export const storageStatePath = resolve(__dirname, '../../../test-results/ui/.auth/state.json')

export function uiEnv(): UiEnv {
  const baseUrl = process.env.E2E_UI_URL ?? process.env.FRONTEND_URL
  if (!baseUrl) throw new Error('Set E2E_UI_URL to the deployment under test')
  const apiKey = process.env.MOATLESS_API_KEY
  if (!apiKey?.startsWith('mvk_')) throw new Error('Set MOATLESS_API_KEY to an mvk_ API key')
  return {
    baseUrl: baseUrl.replace(/\/+$/, ''),
    apiKey,
    project: process.env.E2E_UI_PROJECT ?? 'Moatable',
  }
}

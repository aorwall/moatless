/**
 * Direct database access, for state the API does not expose.
 *
 * Only the `directDatabase` profile has this: it runs `psql` inside the compose
 * stack's postgres container, which needs a Docker daemon on the same host. A
 * test that reaches for it guards on the capability first.
 */

import { execSync } from 'node:child_process'

const POSTGRES_CONTAINER = 'moatless-postgres'
const POSTGRES_USER = process.env.POSTGRES_USER || 'moatless'
const POSTGRES_DB = process.env.POSTGRES_DB || 'moatless'

/**
 * Run one statement and return whatever it printed, unaligned and untitled —
 * so a single-column select reads as a plain value.
 */
export function psql(sql: string): string {
  return execSync(
    `docker exec ${POSTGRES_CONTAINER} psql -U ${POSTGRES_USER} -d ${POSTGRES_DB} -tAc "${sql.replace(/"/g, '\\"')}"`,
    { stdio: 'pipe', timeout: 10_000, encoding: 'utf-8' },
  ).trim()
}

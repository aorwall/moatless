#!/usr/bin/env bun
/**
 * Remove E2E leftovers from a shared environment.
 *
 * A run that finishes cleans up after itself. This is for the runs that do not
 * finish: a killed process leaves its task open, and an open task on the
 * cluster keeps a sandbox pod alive until something removes it.
 *
 * Only resources whose name says a run created them are ever touched, and by
 * default only those from runs that started more than three hours ago — long
 * enough that a sweep can never collide with a run still in flight.
 *
 *   bun tests/sweep.ts --dry-run
 *   bun tests/sweep.ts --older-than 30m
 *   E2E_PROFILE=k8s bun tests/sweep.ts
 */

import { createDriver } from './shared/moat'
import { MoatApi } from './shared/moat-api'
import { describeProfile } from './shared/profile'
import { sweep } from './shared/sweep'

const DEFAULT_MINIMUM_AGE_MS = 3 * 60 * 60 * 1000

async function main(): Promise<void> {
  const args = process.argv.slice(2)
  const dryRun = args.includes('--dry-run')
  const minimumAgeMs = parseAge(flagValue(args, '--older-than')) ?? DEFAULT_MINIMUM_AGE_MS

  console.log(`[sweep] ${describeProfile()}`)
  console.log(
    `[sweep] removing runs older than ${Math.round(minimumAgeMs / 60_000)} minutes${
      dryRun ? ' (dry run)' : ''
    }`,
  )

  const api = new MoatApi(await createDriver())
  const report = await sweep(api, { minimumAgeMs, dryRun })

  if (report.failed.length > 0) {
    for (const failure of report.failed) {
      console.error(`[sweep] could not remove ${failure.kind} ${failure.name}: ${failure.reason}`)
    }
    process.exitCode = 1
  }
}

function flagValue(args: string[], flag: string): string | undefined {
  const index = args.indexOf(flag)
  return index >= 0 ? args[index + 1] : undefined
}

/** `90s`, `30m`, `3h`, `2d` — or a bare number of minutes. */
function parseAge(value: string | undefined): number | null {
  if (!value) {
    return null
  }
  const match = /^(\d+)([smhd]?)$/.exec(value.trim())
  if (!match) {
    throw new Error(`Cannot read '${value}' as a duration. Use forms like 90s, 30m, 3h, 2d.`)
  }
  const amount = Number(match[1])
  const unit = { s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 }[match[2] || 'm']
  return amount * (unit ?? 60_000)
}

main().catch((error: unknown) => {
  console.error(`[sweep] failed: ${error instanceof Error ? error.message : String(error)}`)
  process.exit(1)
})

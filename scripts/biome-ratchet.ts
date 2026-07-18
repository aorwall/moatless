#!/usr/bin/env bun
/**
 * Biome lint ratchet.
 *
 * Biome has no native baseline feature, and enabling the three high-value
 * nursery rules (noFloatingPromises / noMisusedPromises / useExhaustiveSwitchCases)
 * as hard errors is all-or-nothing given the existing violation count. This
 * script grandfathers an enumerated per-(file, rule) baseline that can only
 * shrink: it fails CI when any file exceeds its baselined count (or a file with
 * no baseline entry has any violation), so new debt is blocked while existing
 * debt burns down over time.
 *
 * Usage:
 *   bun scripts/biome-ratchet.ts            # check against biome-baseline.json (fails on regressions)
 *   bun scripts/biome-ratchet.ts --update   # shrink the baseline to match current counts
 *   bun scripts/biome-ratchet.ts --update --init  # write the baseline from scratch
 *
 * The rules must be enabled (at least as "warn") in biome.json for this to see
 * anything — biome:check runs with --diagnostic-level=error, so warnings are
 * reported but never fail that command; this ratchet is what makes them bite.
 */
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

/** Rules governed by the ratchet. Extend this list to grandfather more rules. */
export const RATCHET_RULES = [
  'noFloatingPromises',
  'noMisusedPromises',
  'useExhaustiveSwitchCases',
] as const

/** Paths biome lints for the ratchet (mirrors biome.json's frontend surface). */
const LINT_PATHS = ['apps/frontend/src', 'packages']

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..')
const BASELINE_PATH = join(REPO_ROOT, 'biome-baseline.json')

/** `{ [relativeFilePath]: { [ruleName]: violationCount } }` */
export type Counts = Record<string, Record<string, number>>

interface BiomeDiagnostic {
  category?: string
  location?: { path?: string }
}

/** Extracts the short rule name from a biome category like `lint/nursery/noFloatingPromises`. */
export function ruleFromCategory(category: string | undefined): string | undefined {
  if (!category?.startsWith('lint/')) {
    return undefined
  }
  return category.split('/').at(-1)
}

/** Aggregates biome JSON diagnostics into per-(file, rule) counts, keeping only ratcheted rules. */
export function countDiagnostics(diagnostics: BiomeDiagnostic[]): Counts {
  const ratchetRules = new Set<string>(RATCHET_RULES)
  const counts: Counts = {}
  for (const diagnostic of diagnostics) {
    const rule = ruleFromCategory(diagnostic.category)
    const path = diagnostic.location?.path
    if (!(rule && path) || !ratchetRules.has(rule)) {
      continue
    }
    const forFile = (counts[path] ??= {})
    forFile[rule] = (forFile[rule] ?? 0) + 1
  }
  return counts
}

export interface Entry {
  file: string
  rule: string
  baseline: number
  current: number
}

export interface Verdict {
  /** (file, rule) pairs whose current count exceeds the baseline (baseline missing = 0). */
  regressions: Entry[]
  /** (file, rule) pairs whose current count dropped below the baseline. */
  improvements: Entry[]
  ok: boolean
}

function eachEntry(baseline: Counts, current: Counts): Entry[] {
  const entries: Entry[] = []
  const files = new Set([...Object.keys(baseline), ...Object.keys(current)])
  for (const file of files) {
    const rules = new Set([...Object.keys(baseline[file] ?? {}), ...Object.keys(current[file] ?? {})])
    for (const rule of rules) {
      entries.push({
        file,
        rule,
        baseline: baseline[file]?.[rule] ?? 0,
        current: current[file]?.[rule] ?? 0,
      })
    }
  }
  return entries
}

/** Pure compare: any (file, rule) exceeding its baseline (implicit 0) is a regression. */
export function compareCounts(baseline: Counts, current: Counts): Verdict {
  const regressions: Entry[] = []
  const improvements: Entry[] = []
  for (const entry of eachEntry(baseline, current)) {
    if (entry.current > entry.baseline) {
      regressions.push(entry)
    } else if (entry.current < entry.baseline) {
      improvements.push(entry)
    }
  }
  return { regressions, improvements, ok: regressions.length === 0 }
}

/**
 * Pure update: the baseline may only shrink. Each entry becomes min(current,
 * baseline); entries that reached 0 drop out. Increases (including brand-new
 * violating files) are refused and reported so they must be addressed in the
 * change under review rather than silently baselined.
 */
export function tightenBaseline(
  baseline: Counts,
  current: Counts,
): { updated: Counts; refused: Entry[] } {
  const updated: Counts = {}
  const refused: Entry[] = []
  for (const entry of eachEntry(baseline, current)) {
    const inBaseline = baseline[entry.file]?.[entry.rule] !== undefined
    if (entry.current > entry.baseline) {
      refused.push(entry)
    }
    if (!inBaseline) {
      // New (file, rule) pair: never auto-added — new code must be clean.
      continue
    }
    const next = Math.min(entry.baseline, entry.current)
    if (next > 0) {
      ;(updated[entry.file] ??= {})[entry.rule] = next
    }
  }
  return { updated, refused }
}

/** First-time snapshot: take current counts wholesale (dropping zero entries). */
export function snapshotBaseline(current: Counts): Counts {
  const out: Counts = {}
  for (const [file, rules] of Object.entries(current)) {
    for (const [rule, count] of Object.entries(rules)) {
      if (count > 0) {
        ;(out[file] ??= {})[rule] = count
      }
    }
  }
  return out
}

/** Serialize a baseline deterministically (sorted keys) for stable diffs. */
export function serializeBaseline(counts: Counts): string {
  const sorted: Counts = {}
  for (const file of Object.keys(counts).sort()) {
    const rules = counts[file]
    if (!rules) {
      continue
    }
    const sortedRules: Record<string, number> = {}
    for (const rule of Object.keys(rules).sort()) {
      const count = rules[rule]
      if (count !== undefined) {
        sortedRules[rule] = count
      }
    }
    sorted[file] = sortedRules
  }
  return `${JSON.stringify(sorted, null, 2)}\n`
}

function loadBaseline(): Counts {
  if (!existsSync(BASELINE_PATH)) {
    return {}
  }
  return JSON.parse(readFileSync(BASELINE_PATH, 'utf8')) as Counts
}

/** Runs biome and returns parsed diagnostics plus the list of files biome could not analyze. */
function runBiome(): { diagnostics: BiomeDiagnostic[]; panickedFiles: string[] } {
  const result = spawnSync(
    'bunx',
    ['biome', 'lint', '--reporter=json', '--max-diagnostics=100000', ...LINT_PATHS],
    { cwd: REPO_ROOT, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
  )
  if (!result.stdout) {
    throw new Error(`biome produced no output.\n${result.stderr ?? ''}`)
  }
  let parsed: { diagnostics?: BiomeDiagnostic[] }
  try {
    parsed = JSON.parse(result.stdout)
  } catch (error) {
    throw new Error(`could not parse biome JSON output: ${(error as Error).message}`)
  }
  const diagnostics = parsed.diagnostics ?? []
  // Biome 2.5.3 panics on a handful of files (a known module-graph bug); those
  // files are silently skipped, so the ratchet's coverage of them is nil.
  const panickedFiles = diagnostics
    .filter((d) => d.category === 'internalError/panic')
    .map((d) => d.location?.path)
    .filter((p): p is string => Boolean(p))
  return { diagnostics, panickedFiles }
}

function reportEntries(label: string, entries: Entry[]): void {
  console.error(`\n${label}`)
  for (const e of entries.sort((a, b) => a.file.localeCompare(b.file))) {
    console.error(`  ${e.file}  ${e.rule}: ${e.baseline} → ${e.current}`)
  }
}

function main(): void {
  const args = new Set(process.argv.slice(2))
  const isUpdate = args.has('--update')
  const isInit = args.has('--init')

  const { diagnostics, panickedFiles } = runBiome()
  const current = countDiagnostics(diagnostics)

  if (panickedFiles.length > 0) {
    console.error(
      `⚠ biome could not analyze ${panickedFiles.length} file(s) (internal panic); ` +
        'ratchet coverage of them is partial:',
    )
    for (const file of [...new Set(panickedFiles)].sort()) {
      console.error(`    ${file}`)
    }
  }

  if (isUpdate) {
    const baseline = isInit ? {} : loadBaseline()
    const { updated, refused } = isInit
      ? { updated: snapshotBaseline(current), refused: [] as Entry[] }
      : tightenBaseline(baseline, current)
    writeFileSync(BASELINE_PATH, serializeBaseline(updated))
    console.error(`✓ wrote ${BASELINE_PATH}`)
    if (refused.length > 0) {
      reportEntries(
        '✗ refused to raise the baseline for these entries — fix them instead of baselining:',
        refused,
      )
      process.exit(1)
    }
    return
  }

  const baseline = loadBaseline()
  const verdict = compareCounts(baseline, current)

  if (verdict.improvements.length > 0) {
    reportEntries(
      'ℹ ratcheted rules improved below baseline — run `bun run lint:ratchet:update` to lock it in:',
      verdict.improvements,
    )
  }

  if (!verdict.ok) {
    reportEntries('✗ new ratcheted-rule violations exceed the baseline:', verdict.regressions)
    console.error(
      '\nFix the violations above (preferred) or, if unavoidable, edit biome-baseline.json in review.',
    )
    process.exit(1)
  }

  console.error('✓ no new ratcheted-rule violations')
}

if (import.meta.main) {
  main()
}

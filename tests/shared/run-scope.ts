/**
 * Naming that lets a run coexist with real data.
 *
 * The deployed environment's database is shared and long-lived: there is no
 * truncate to fall back on, and no tenancy boundary to hide behind — resource
 * scope is only global or per-user, and teams grant no access (ADR-0006). So
 * the suite enforces the convention on itself. Every resource a test creates is
 * named through `scopedName`, which stamps it with the run's own prefix.
 *
 * The prefix carries the run's start time, so a sweep can decide what is stale
 * from the name alone — no database access, no bookkeeping file, nothing that
 * a killed run could fail to write.
 *
 *   e2e-mfz3k1p-a4f9-Task Filter Repo
 *   └─┬─┘└──┬──┘└─┬─┘
 *     │     │     └── random, so two runs starting in the same millisecond differ
 *     │     └──────── start time, base 36
 *     └────────────── the family every run shares
 */

/** The marker every run's resources carry. */
export const RUN_FAMILY = 'e2e'

const PREFIX_PATTERN = /^e2e-([0-9a-z]+)-([0-9a-z]{4})/

let cachedPrefix: string | null = null

/**
 * This run's prefix, minted once per process.
 *
 * `E2E_RUN_PREFIX` overrides it, so a sweep — or a second process joining a run
 * already in flight — can name the same run.
 */
export function runPrefix(): string {
  if (!cachedPrefix) {
    cachedPrefix =
      process.env.E2E_RUN_PREFIX?.trim() ||
      `${RUN_FAMILY}-${Date.now().toString(36)}-${randomSuffix()}`
  }
  return cachedPrefix
}

/**
 * A resource name belonging to this run.
 *
 * Idempotent: the domain verbs stamp every name they send, so a test that also
 * stamps one — because it asserts on the name later — gets the same string back
 * rather than a doubled prefix.
 */
export function scopedName(label: string): string {
  return belongsToThisRun(label) ? label : `${runPrefix()}-${label}`
}

/** A slug belonging to this run, for resources that need one (teams). */
export function scopedSlug(label: string): string {
  return scopedName(label)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
}

/** Whether a name was created by this run. */
export function belongsToThisRun(name: string | null | undefined): boolean {
  return typeof name === 'string' && name.startsWith(`${runPrefix()}-`)
}

/** Whether a name was created by any run of the suite. */
export function belongsToAnyRun(name: string | null | undefined): boolean {
  return typeof name === 'string' && PREFIX_PATTERN.test(name)
}

/**
 * When the run that created `name` started, or null when the name is not one of
 * ours. A name we cannot read the time from is not ours — a sweep must never
 * guess.
 */
export function runStartedAt(name: string | null | undefined): Date | null {
  if (typeof name !== 'string') {
    return null
  }
  const match = PREFIX_PATTERN.exec(name)
  if (!match) {
    return null
  }
  const millis = Number.parseInt(match[1], 36)
  return Number.isFinite(millis) && millis > 0 ? new Date(millis) : null
}

/** Keep only the items this run created. */
export function ownedByThisRun<T>(items: T[], nameOf: (item: T) => string | null | undefined): T[] {
  return items.filter((item) => belongsToThisRun(nameOf(item)))
}

function randomSuffix(): string {
  return Math.random().toString(36).slice(2, 6).padEnd(4, '0')
}

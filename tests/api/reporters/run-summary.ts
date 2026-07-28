/**
 * End-of-run summary: what ran, what did not, and why not.
 *
 * The built-in reporters count skips. On a suite that runs against two
 * environments with different capabilities that is not enough — a run where a
 * whole tier silently stopped executing looks exactly like a healthy one if all
 * you see is "41 skipped". So the skips are grouped by their reason and printed
 * with the tests they cover, and the profile that produced them is named above.
 *
 * A skipped test has to be as visible as a failing one, or coverage erodes
 * without anybody deciding to let it.
 */

import type {
  FullConfig,
  FullResult,
  Reporter,
  Suite,
  TestCase,
  TestResult,
} from '@playwright/test/reporter'
import { describeProfile } from '../../shared/profile'

/**
 * Why a test did not run.
 *
 * Two different things arrive as "skipped". A test the profile cannot run
 * carries the `test.skip(condition, reason)` annotation naming the missing
 * capability. A test in a serial chain whose earlier step failed carries
 * nothing — it was never started. Reporting both as one number is how a chain
 * that collapsed on its second step reads as an environment that simply cannot
 * run it.
 */
function skipReason(test: TestCase): string {
  const annotation = test.annotations.find((entry) => entry.type === 'skip')
  if (!annotation) {
    return 'not reached — an earlier step in the chain failed'
  }
  return annotation.description?.trim() || 'no reason given'
}

/**
 * The assertion that failed, in one line.
 *
 * A failure named only by its test tells whoever reads the summary — a person
 * at a terminal or the nightly Loop writing its report — nothing they can act
 * on without re-running. Playwright puts the `expect(value, "message")` text on
 * the first line of the error, which is exactly the sentence the assertion was
 * written to say.
 */
function failureReason(result: TestResult): string {
  const raw = result.error?.message ?? result.errors[0]?.message ?? ''
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping the reporter's own ANSI colours
  const plain = raw.replace(/\u001B\[[0-9;]*m/g, '')
  const first = plain
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
  if (!first) {
    return result.status === 'timedOut' ? 'timed out' : 'no error message'
  }
  return first.replace(/^Error:\s*/, '')
}

export default class RunSummary implements Reporter {
  private readonly skipped = new Map<string, string[]>()
  private readonly failed: Array<{ title: string; reason: string }> = []
  private passed = 0
  private prefix = ''

  onBegin(config: FullConfig, _suite: Suite): void {
    this.prefix = String((config.metadata as { runPrefix?: string }).runPrefix ?? '')
  }

  onTestEnd(test: TestCase, result: TestResult): void {
    if (result.status === 'skipped') {
      const reason = skipReason(test)
      const tests = this.skipped.get(reason) ?? []
      tests.push(test.title)
      this.skipped.set(reason, tests)
      return
    }
    if (result.status === 'passed') {
      this.passed += 1
      return
    }
    this.failed.push({
      title: `${test.parent.title} › ${test.title}`,
      reason: failureReason(result),
    })
  }

  onEnd(result: FullResult): void {
    const skippedCount = [...this.skipped.values()].reduce(
      (total, tests) => total + tests.length,
      0,
    )

    console.log(`\n${'─'.repeat(72)}`)
    console.log(`Run ${this.prefix} — ${describeProfile()}`)
    console.log(
      `${this.passed} passed, ${this.failed.length} failed, ${skippedCount} skipped (${result.status})`,
    )

    if (this.skipped.size > 0) {
      console.log('\nDid not run:')
      // Longest list first — the biggest gap in coverage is the one worth
      // reading before the others.
      const byCount = [...this.skipped.entries()].sort((a, b) => b[1].length - a[1].length)
      for (const [reason, tests] of byCount) {
        console.log(`  ${tests.length} × ${reason}`)
        for (const title of tests) {
          console.log(`      ${title}`)
        }
      }
    }

    if (this.failed.length > 0) {
      console.log('\nFailed:')
      for (const failure of this.failed) {
        console.log(`  ${failure.title}`)
        console.log(`      ${failure.reason}`)
      }
    }
    console.log(`${'─'.repeat(72)}\n`)
  }
}

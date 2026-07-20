import { describe, expect, it } from 'vitest'
import {
  compareCounts,
  countDiagnostics,
  ruleFromCategory,
  serializeBaseline,
  snapshotBaseline,
  tightenBaseline,
} from '../biome-ratchet'

describe('ruleFromCategory', () => {
  it('extracts the short rule name from a lint category', () => {
    expect(ruleFromCategory('lint/nursery/noFloatingPromises')).toBe('noFloatingPromises')
  })

  it('ignores non-lint categories (e.g. internal panics)', () => {
    expect(ruleFromCategory('internalError/panic')).toBeUndefined()
    expect(ruleFromCategory(undefined)).toBeUndefined()
  })
})

describe('countDiagnostics', () => {
  it('aggregates only ratcheted rules per (file, rule)', () => {
    const counts = countDiagnostics([
      { category: 'lint/nursery/noFloatingPromises', location: { path: 'a.ts' } },
      { category: 'lint/nursery/noFloatingPromises', location: { path: 'a.ts' } },
      { category: 'lint/nursery/noMisusedPromises', location: { path: 'a.ts' } },
      // Not a ratcheted rule — ignored.
      { category: 'lint/style/noNonNullAssertion', location: { path: 'a.ts' } },
      // Internal panic — ignored.
      { category: 'internalError/panic', location: { path: 'b.ts' } },
    ])
    expect(counts).toEqual({ 'a.ts': { noFloatingPromises: 2, noMisusedPromises: 1 } })
  })
})

describe('compareCounts', () => {
  it('passes when current matches the baseline', () => {
    const baseline = { 'a.ts': { noFloatingPromises: 2 } }
    const verdict = compareCounts(baseline, { 'a.ts': { noFloatingPromises: 2 } })
    expect(verdict.ok).toBe(true)
    expect(verdict.regressions).toEqual([])
    expect(verdict.improvements).toEqual([])
  })

  it('flags a count that exceeds the baseline', () => {
    const verdict = compareCounts(
      { 'a.ts': { noFloatingPromises: 1 } },
      { 'a.ts': { noFloatingPromises: 3 } },
    )
    expect(verdict.ok).toBe(false)
    expect(verdict.regressions).toEqual([
      { file: 'a.ts', rule: 'noFloatingPromises', baseline: 1, current: 3 },
    ])
  })

  it('treats a file absent from the baseline as an implicit 0 (new files must be clean)', () => {
    const verdict = compareCounts({}, { 'new.ts': { noFloatingPromises: 1 } })
    expect(verdict.ok).toBe(false)
    expect(verdict.regressions).toEqual([
      { file: 'new.ts', rule: 'noFloatingPromises', baseline: 0, current: 1 },
    ])
  })

  it('reports improvements without failing', () => {
    const verdict = compareCounts(
      { 'a.ts': { noFloatingPromises: 5 } },
      { 'a.ts': { noFloatingPromises: 2 } },
    )
    expect(verdict.ok).toBe(true)
    expect(verdict.improvements).toEqual([
      { file: 'a.ts', rule: 'noFloatingPromises', baseline: 5, current: 2 },
    ])
  })
})

describe('tightenBaseline', () => {
  it('shrinks counts and drops entries that reached zero', () => {
    const { updated, refused } = tightenBaseline(
      { 'a.ts': { noFloatingPromises: 5, noMisusedPromises: 2 } },
      { 'a.ts': { noFloatingPromises: 3 } },
    )
    // noFloatingPromises shrank 5→3; noMisusedPromises reached 0 and is dropped.
    expect(updated).toEqual({ 'a.ts': { noFloatingPromises: 3 } })
    expect(refused).toEqual([])
  })

  it('refuses to raise a count above the baseline and leaves it unchanged', () => {
    const { updated, refused } = tightenBaseline(
      { 'a.ts': { noFloatingPromises: 1 } },
      { 'a.ts': { noFloatingPromises: 4 } },
    )
    expect(updated).toEqual({ 'a.ts': { noFloatingPromises: 1 } })
    expect(refused).toEqual([
      { file: 'a.ts', rule: 'noFloatingPromises', baseline: 1, current: 4 },
    ])
  })

  it('refuses to add a brand-new violating file', () => {
    const { updated, refused } = tightenBaseline({}, { 'new.ts': { noFloatingPromises: 2 } })
    expect(updated).toEqual({})
    expect(refused).toEqual([
      { file: 'new.ts', rule: 'noFloatingPromises', baseline: 0, current: 2 },
    ])
  })
})

describe('snapshotBaseline', () => {
  it('takes current counts wholesale, dropping zero entries', () => {
    expect(
      snapshotBaseline({ 'a.ts': { noFloatingPromises: 2, noMisusedPromises: 0 } }),
    ).toEqual({ 'a.ts': { noFloatingPromises: 2 } })
  })
})

describe('serializeBaseline', () => {
  it('emits deterministically sorted JSON with a trailing newline', () => {
    const out = serializeBaseline({
      'b.ts': { noMisusedPromises: 1 },
      'a.ts': { noMisusedPromises: 1, noFloatingPromises: 2 },
    })
    expect(out.endsWith('\n')).toBe(true)
    // Files sorted (a before b) and rules sorted within a file.
    expect(out).toBe(
      `${JSON.stringify(
        {
          'a.ts': { noFloatingPromises: 2, noMisusedPromises: 1 },
          'b.ts': { noMisusedPromises: 1 },
        },
        null,
        2,
      )}\n`,
    )
  })
})

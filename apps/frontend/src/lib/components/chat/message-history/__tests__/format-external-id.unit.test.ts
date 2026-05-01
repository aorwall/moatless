import { describe, expect, it } from 'vitest'
import { formatExternalId } from '../channel-message'

describe('formatExternalId', () => {
  it('formats github issue/PR comments', () => {
    expect(formatExternalId('github:ic:12345')).toBe('Comment #12345')
  })

  it('formats github review comments', () => {
    expect(formatExternalId('github:rc:99')).toBe('Review #99')
  })

  it('formats github reviews', () => {
    expect(formatExternalId('github:rv:7')).toBe('Review #7')
  })

  it('formats github PR action without sha', () => {
    expect(formatExternalId('github:pr:opened:42')).toBe('PR #42 opened')
  })

  it('formats github PR action with sha', () => {
    expect(formatExternalId('github:pr:synchronize:42:abc1234')).toBe('PR #42 synchronize abc1234')
  })

  it('formats github check runs', () => {
    expect(formatExternalId('github:cr:55')).toBe('Check #55')
  })

  it('formats github check suites', () => {
    expect(formatExternalId('github:cs:66')).toBe('Suite #66')
  })

  it('formats slack timestamps', () => {
    expect(formatExternalId('slack:ts:1700000.000123')).toBe('ts 1700000.000123')
  })

  it('formats telegram messages', () => {
    expect(formatExternalId('tg:m:789')).toBe('msg #789')
  })

  it('falls back to raw value for legacy bare integer', () => {
    expect(formatExternalId('12345')).toBe('12345')
  })

  it('falls back to raw value for unknown namespace', () => {
    expect(formatExternalId('discord:msg:42')).toBe('discord:msg:42')
  })

  it('falls back to raw value for unknown github kind', () => {
    expect(formatExternalId('github:unknown:42')).toBe('github:unknown:42')
  })
})

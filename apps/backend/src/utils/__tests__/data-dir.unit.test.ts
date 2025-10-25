import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getDataDir, getDefaultDataDir } from '../data-dir'

describe('data-dir utilities', () => {
  const originalDataDir = process.env['DATA_DIR']

  beforeEach(() => {
    // Reset DATA_DIR before each test
    Reflect.deleteProperty(process.env, 'DATA_DIR')
  })

  afterEach(() => {
    // Restore original DATA_DIR
    if (originalDataDir) {
      process.env['DATA_DIR'] = originalDataDir
    } else {
      Reflect.deleteProperty(process.env, 'DATA_DIR')
    }
  })

  describe('getDataDir', () => {
    it('returns DATA_DIR when environment variable is set', () => {
      const testDir = '/tmp/test-data'
      process.env['DATA_DIR'] = testDir

      expect(getDataDir()).toBe(testDir)
    })

    it('returns default directory when DATA_DIR is not set', () => {
      Reflect.deleteProperty(process.env, 'DATA_DIR')

      const expected = path.join(os.homedir(), '.moatless')
      expect(getDataDir()).toBe(expected)
    })

    it('returns default directory when DATA_DIR is empty', () => {
      process.env['DATA_DIR'] = ''

      const expected = path.join(os.homedir(), '.moatless')
      expect(getDataDir()).toBe(expected)
    })
  })

  describe('getDefaultDataDir', () => {
    it('always returns ~/.moatless regardless of environment', () => {
      const expected = path.join(os.homedir(), '.moatless')

      // Test with DATA_DIR set
      process.env['DATA_DIR'] = '/some/other/path'
      expect(getDefaultDataDir()).toBe(expected)

      // Test with DATA_DIR unset
      Reflect.deleteProperty(process.env, 'DATA_DIR')
      expect(getDefaultDataDir()).toBe(expected)
    })
  })

  describe('test environment integration', () => {
    it('uses temporary directory when DATA_DIR is set by test setup', () => {
      // Simulate test setup setting DATA_DIR to a temporary directory
      const tempDir = '/tmp/moatless-test-12345'
      process.env['DATA_DIR'] = tempDir

      expect(getDataDir()).toBe(tempDir)
    })
  })
})

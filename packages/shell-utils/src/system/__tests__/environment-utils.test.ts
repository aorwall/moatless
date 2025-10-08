import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getSafeSystemEnvironment } from '../environment-utils.js'

describe('environment-utils', () => {
  describe('getSafeSystemEnvironment', () => {
    const originalEnv = process.env

    beforeEach(() => {
      // Create a clean environment for each test
      process.env = { ...originalEnv }
    })

    afterEach(() => {
      // Restore original environment
      process.env = originalEnv
    })

    it('should include PATH if set', () => {
      process.env.PATH = '/usr/bin:/bin'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.PATH).toBe('/usr/bin:/bin')
    })

    it('should include HOME if set', () => {
      process.env.HOME = '/home/user'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.HOME).toBe('/home/user')
    })

    it('should include USER if set', () => {
      process.env.USER = 'testuser'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.USER).toBe('testuser')
    })

    it('should include shell-related variables', () => {
      process.env.SHELL = '/bin/bash'
      process.env.TERM = 'xterm-256color'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.SHELL).toBe('/bin/bash')
      expect(safeEnv.TERM).toBe('xterm-256color')
    })

    it('should include NVM variables if set', () => {
      process.env.NVM_DIR = '/home/user/.nvm'
      process.env.NVM_BIN = '/home/user/.nvm/bin'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.NVM_DIR).toBe('/home/user/.nvm')
      expect(safeEnv.NVM_BIN).toBe('/home/user/.nvm/bin')
    })

    it('should include version manager variables', () => {
      process.env.PYENV_ROOT = '/home/user/.pyenv'
      process.env.RBENV_ROOT = '/home/user/.rbenv'
      process.env.SDKMAN_DIR = '/home/user/.sdkman'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.PYENV_ROOT).toBe('/home/user/.pyenv')
      expect(safeEnv.RBENV_ROOT).toBe('/home/user/.rbenv')
      expect(safeEnv.SDKMAN_DIR).toBe('/home/user/.sdkman')
    })

    it('should include CI/CD detection variables', () => {
      process.env.CI = 'true'
      process.env.CONTINUOUS_INTEGRATION = 'true'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.CI).toBe('true')
      expect(safeEnv.CONTINUOUS_INTEGRATION).toBe('true')
    })

    it('should not include undefined variables', () => {
      process.env.SOME_RANDOM_VAR = undefined
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.SOME_RANDOM_VAR).toBeUndefined()
    })

    it('should not include unsafe/custom variables', () => {
      process.env.API_KEY = 'secret'
      process.env.DATABASE_URL = 'postgres://...'
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv.API_KEY).toBeUndefined()
      expect(safeEnv.DATABASE_URL).toBeUndefined()
    })

    it('should return an object with only string values', () => {
      process.env.PATH = '/usr/bin'
      process.env.HOME = '/home/user'
      const safeEnv = getSafeSystemEnvironment()

      for (const [key, value] of Object.entries(safeEnv)) {
        expect(typeof key).toBe('string')
        expect(typeof value).toBe('string')
      }
    })

    it('should return a new object each time', () => {
      const env1 = getSafeSystemEnvironment()
      const env2 = getSafeSystemEnvironment()
      expect(env1).not.toBe(env2) // Different object references
      expect(env1).toEqual(env2) // Same content
    })

    it('should work with minimal environment', () => {
      process.env = {}
      const safeEnv = getSafeSystemEnvironment()
      expect(safeEnv).toEqual({})
    })

    it('should work with full environment', () => {
      process.env = {
        PATH: '/usr/bin',
        HOME: '/home/user',
        USER: 'testuser',
        SHELL: '/bin/bash',
        TERM: 'xterm',
        LANG: 'en_US.UTF-8',
        NVM_DIR: '/home/user/.nvm',
        TMPDIR: '/tmp',
        CI: 'true',
      }

      const safeEnv = getSafeSystemEnvironment()

      expect(safeEnv.PATH).toBe('/usr/bin')
      expect(safeEnv.HOME).toBe('/home/user')
      expect(safeEnv.USER).toBe('testuser')
      expect(safeEnv.SHELL).toBe('/bin/bash')
      expect(safeEnv.TERM).toBe('xterm')
      expect(safeEnv.LANG).toBe('en_US.UTF-8')
      expect(safeEnv.NVM_DIR).toBe('/home/user/.nvm')
      expect(safeEnv.TMPDIR).toBe('/tmp')
      expect(safeEnv.CI).toBe('true')
    })
  })
})

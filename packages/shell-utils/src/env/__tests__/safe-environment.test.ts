import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getSafeSystemEnvironment } from '../safe-environment.js'

describe('getSafeSystemEnvironment', () => {
  const originalEnv = process.env

  beforeEach(() => {
    // Reset process.env to a clean state for each test
    process.env = { ...originalEnv }
  })

  afterEach(() => {
    process.env = originalEnv
  })

  it('should include essential system variables when present', () => {
    process.env['PATH'] = '/usr/bin:/bin'
    process.env['HOME'] = '/home/user'
    process.env['USER'] = 'testuser'
    process.env['SHELL'] = '/bin/bash'
    process.env['TERM'] = 'xterm-256color'
    process.env['LANG'] = 'en_US.UTF-8'
    process.env['TMPDIR'] = '/tmp'

    const env = getSafeSystemEnvironment()

    expect(env['PATH']).toBe('/usr/bin:/bin')
    expect(env['HOME']).toBe('/home/user')
    expect(env['USER']).toBe('testuser')
    expect(env['SHELL']).toBe('/bin/bash')
    expect(env['TERM']).toBe('xterm-256color')
    expect(env['LANG']).toBe('en_US.UTF-8')
    expect(env['TMPDIR']).toBe('/tmp')
  })

  it('should include version manager variables when present', () => {
    process.env['NVM_DIR'] = '/home/user/.nvm'
    process.env['NVM_BIN'] = '/home/user/.nvm/versions/node/v18/bin'
    process.env['BUN_INSTALL'] = '/home/user/.bun'
    process.env['VOLTA_HOME'] = '/home/user/.volta'
    process.env['FNM_DIR'] = '/home/user/.fnm'
    process.env['N_PREFIX'] = '/home/user/n'

    const env = getSafeSystemEnvironment()

    expect(env['NVM_DIR']).toBe('/home/user/.nvm')
    expect(env['NVM_BIN']).toBe('/home/user/.nvm/versions/node/v18/bin')
    expect(env['BUN_INSTALL']).toBe('/home/user/.bun')
    expect(env['VOLTA_HOME']).toBe('/home/user/.volta')
    expect(env['FNM_DIR']).toBe('/home/user/.fnm')
    expect(env['N_PREFIX']).toBe('/home/user/n')
  })

  it('should include XDG variables when present', () => {
    process.env['XDG_CONFIG_HOME'] = '/home/user/.config'
    process.env['XDG_DATA_HOME'] = '/home/user/.local/share'
    process.env['XDG_CACHE_HOME'] = '/home/user/.cache'

    const env = getSafeSystemEnvironment()

    expect(env['XDG_CONFIG_HOME']).toBe('/home/user/.config')
    expect(env['XDG_DATA_HOME']).toBe('/home/user/.local/share')
    expect(env['XDG_CACHE_HOME']).toBe('/home/user/.cache')
  })

  it('should include CI variable when present', () => {
    process.env['CI'] = 'true'

    const env = getSafeSystemEnvironment()

    expect(env['CI']).toBe('true')
  })

  it('should exclude sensitive variables', () => {
    process.env['ANTHROPIC_API_KEY'] = 'sk-ant-secret'
    process.env['CLAUDE_CODE_OAUTH_TOKEN'] = 'oauth-token'
    process.env['NODE_ENV'] = 'production'
    process.env['DATABASE_URL'] = 'postgres://...'
    process.env['AWS_SECRET_ACCESS_KEY'] = 'aws-secret'

    const env = getSafeSystemEnvironment()

    expect(env['ANTHROPIC_API_KEY']).toBeUndefined()
    expect(env['CLAUDE_CODE_OAUTH_TOKEN']).toBeUndefined()
    expect(env['NODE_ENV']).toBeUndefined()
    expect(env['DATABASE_URL']).toBeUndefined()
    expect(env['AWS_SECRET_ACCESS_KEY']).toBeUndefined()
  })

  it('should omit variables that are not set', () => {
    process.env['NVM_DIR'] = undefined
    process.env['VOLTA_HOME'] = undefined
    process.env['CI'] = undefined

    const env = getSafeSystemEnvironment()

    expect('NVM_DIR' in env).toBe(false)
    expect('VOLTA_HOME' in env).toBe(false)
    expect('CI' in env).toBe(false)
  })

  it('should respect SHELL_COMMAND_PATH override for PATH', () => {
    process.env['PATH'] = '/usr/bin:/bin'
    process.env['SHELL_COMMAND_PATH'] = '/custom/path:/other/path'

    const env = getSafeSystemEnvironment()

    expect(env['PATH']).toBe('/custom/path:/other/path')
  })

  it('should use regular PATH when SHELL_COMMAND_PATH is not set', () => {
    process.env['PATH'] = '/usr/bin:/bin'
    process.env['SHELL_COMMAND_PATH'] = undefined

    const env = getSafeSystemEnvironment()

    expect(env['PATH']).toBe('/usr/bin:/bin')
  })

  it('should return only string values (no undefined)', () => {
    const env = getSafeSystemEnvironment()

    for (const value of Object.values(env)) {
      expect(typeof value).toBe('string')
    }
  })
})

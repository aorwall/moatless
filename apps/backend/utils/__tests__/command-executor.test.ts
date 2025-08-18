import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { type CommandExecutionOptions, executeCommandWithStream } from '../command-executor.js'

describe('executeCommandWithStream', () => {
  let tempDir: string
  let testOptions: CommandExecutionOptions

  beforeEach(async () => {
    // Create a temporary directory for each test
    tempDir = await fs.mkdtemp(path.join(tmpdir(), 'command-executor-test-'))
    testOptions = {
      cwd: tempDir,
      timeout: 5000,
    }
  })

  afterEach(async () => {
    // Clean up temporary directory
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (error) {
      console.warn('Failed to clean up temp dir:', error)
    }
  })

  it('should execute a simple command successfully', async () => {
    const events = []

    for await (const event of executeCommandWithStream('echo "hello world"', testOptions)) {
      events.push(event)
    }

    expect(events).toHaveLength(3) // start, output, complete
    expect(events[0].type).toBe('start')
    expect(events[0].data.command).toBe('echo "hello world"')

    expect(events[1].type).toBe('output')
    expect(events[1].data.text).toContain('hello world')
    expect(events[1].data.outputType).toBe('stdout')

    expect(events[2].type).toBe('complete')
    expect(events[2].data.exitCode).toBe(0)
    expect(events[2].data.error).toBeUndefined()
  })

  it('should execute multiple commands with &&', async () => {
    const events = []

    for await (const event of executeCommandWithStream(
      'echo "first" && echo "second"',
      testOptions,
    )) {
      events.push(event)
    }

    expect(events.length).toBeGreaterThanOrEqual(3) // start, output(s), complete
    expect(events[0].type).toBe('start')

    const outputEvents = events.filter((e) => e.type === 'output')
    const allOutput = outputEvents.map((e) => e.data.text).join('')
    expect(allOutput).toContain('first')
    expect(allOutput).toContain('second')

    const completeEvent = events[events.length - 1]
    expect(completeEvent.type).toBe('complete')
    expect(completeEvent.data.exitCode).toBe(0)
  })

  it('should handle command execution errors', async () => {
    const events = []

    for await (const event of executeCommandWithStream('nonexistentcommand', testOptions)) {
      events.push(event)
    }

    expect(events.length).toBeGreaterThanOrEqual(2) // start, [stderr], complete
    expect(events[0].type).toBe('start')

    const completeEvent = events[events.length - 1]
    expect(completeEvent.type).toBe('complete')
    expect(completeEvent.data.exitCode).not.toBe(0)
  })

  it('should handle stderr output', async () => {
    const events = []

    // Command that writes to stderr
    for await (const event of executeCommandWithStream('echo "error message" >&2', testOptions)) {
      events.push(event)
    }

    expect(events.length).toBeGreaterThanOrEqual(2) // start, stderr output, complete

    const stderrEvents = events.filter((e) => e.type === 'output' && e.data.outputType === 'stderr')
    expect(stderrEvents.length).toBeGreaterThan(0)
    expect(stderrEvents[0].data.text).toContain('error message')
  })

  it('should timeout long-running commands', async () => {
    const events = []
    const shortTimeout = { ...testOptions, timeout: 1000 }

    // Command that sleeps for longer than timeout
    for await (const event of executeCommandWithStream('sleep 2', shortTimeout)) {
      events.push(event)
    }

    const completeEvent = events[events.length - 1]
    expect(completeEvent.type).toBe('complete')
    expect(completeEvent.data.error).toBe('Command timed out')
  })

  it('should work with nvm commands', async () => {
    const events = []

    // Test nvm loading (should work even if nvm isn't installed)
    for await (const event of executeCommandWithStream('node --version', testOptions)) {
      events.push(event)
    }

    expect(events[0].type).toBe('start')
    const completeEvent = events[events.length - 1]
    expect(completeEvent.type).toBe('complete')

    // Should complete (either successfully if node is available, or with error if not)
    expect([0, 1]).toContain(completeEvent.data.exitCode)
  })

  it('should set working directory correctly', async () => {
    // Create a test file in the temp directory
    const testFile = path.join(tempDir, 'test.txt')
    await fs.writeFile(testFile, 'test content')

    const events = []

    for await (const event of executeCommandWithStream('ls test.txt', testOptions)) {
      events.push(event)
    }

    const outputEvents = events.filter((e) => e.type === 'output')
    const allOutput = outputEvents.map((e) => e.data.text).join('')
    expect(allOutput).toContain('test.txt')

    const completeEvent = events[events.length - 1]
    expect(completeEvent.data.exitCode).toBe(0)
  })

  it('should handle environment variables', async () => {
    const events = []
    const envOptions = {
      ...testOptions,
      env: { TEST_VAR: 'test_value' },
    }

    for await (const event of executeCommandWithStream('echo $TEST_VAR', envOptions)) {
      events.push(event)
    }

    const outputEvents = events.filter((e) => e.type === 'output')
    const allOutput = outputEvents.map((e) => e.data.text).join('')
    expect(allOutput).toContain('test_value')
  })
})

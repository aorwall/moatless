/**
 * The moat driver — every API call the suite makes goes through the compiled
 * `moat` binary, so the tests exercise the client people actually run.
 *
 * This module owns invocation, JSON parsing and error mapping, and nothing
 * else. Domain verbs live in `tests/api/steps/`; what a given environment looks
 * like lives in `profile.ts`.
 *
 * Authentication is the profile's business too:
 *   - `ambient` — the process is inside a sandbox and is already logged in.
 *   - `password` — log in once, mint an API key, and hand it to every
 *     invocation. Local password auth is the only environment where a test can
 *     create an identity at all.
 */

import { spawn } from 'node:child_process'
import { type Profile, profile } from './profile'

/** Where the binary lives. Overridable for a locally built CLI. */
const MOAT_BIN = process.env.MOAT_BIN || 'moat'

/** Ceiling for a single invocation. Long polls stream instead. */
const DEFAULT_TIMEOUT_MS = 120_000

/** A raw HTTP response, for tests that assert on how a request was rejected. */
export interface RawResponse {
  status: number
  body: unknown
}

/** A running stream. Stopping it is the caller's job. */
export interface StreamHandle {
  stop: () => void
}

/** A `moat` invocation that failed. */
export class MoatError extends Error {
  readonly command: string
  readonly exitCode: number | null
  readonly stdout: string
  readonly stderr: string

  constructor(
    message: string,
    details: {
      command: string
      exitCode: number | null
      stdout: string
      stderr: string
    },
  ) {
    super(message)
    this.name = 'MoatError'
    this.command = details.command
    this.exitCode = details.exitCode
    this.stdout = details.stdout
    this.stderr = details.stderr
  }
}

interface RunResult {
  stdout: string
  stderr: string
  exitCode: number | null
}

interface RunOptions {
  /** Extra environment for this invocation only. */
  env?: Record<string, string>
  /** Text written to the process's stdin. */
  stdin?: string
  /**
   * How long the request itself may take.
   *
   * Some endpoints do real work before they answer — closing a task removes
   * its sandbox, sending a message to a stopped one provisions the
   * replacement — and against a cluster that outruns the CLI's own ceiling.
   * Raising it here raises the CLI's ceiling and the subprocess's together, so
   * the process always outlives the request it is waiting on.
   */
  timeoutMs?: number
}

/**
 * A `moat` invocation, authenticated as one identity.
 *
 * `Moat` is the transport. It exposes three shapes: `json` for a command whose
 * output is parsed, `text` for one whose output is read as-is, and `raw` for a
 * direct request whose status matters — the escape hatch for asserting on how
 * the API rejects something.
 */
export class Moat {
  private readonly baseEnv: Record<string, string>

  constructor(
    private readonly p: Profile,
    env: Record<string, string> = {},
  ) {
    this.baseEnv = { MOATLESS_API_URL: p.baseUrl, ...env }
  }

  /** The environment under test. */
  get profile(): Profile {
    return this.p
  }

  /** Run a command and parse its JSON output. */
  async json<T>(args: string[], options?: RunOptions): Promise<T> {
    const stdout = await this.text(args, options)
    if (stdout.trim() === '') {
      return undefined as T
    }
    try {
      return JSON.parse(stdout) as T
    } catch (error) {
      throw new MoatError(
        `moat ${args.join(' ')} produced output that is not JSON: ${describe(error)}`,
        { command: describeCommand(args), exitCode: 0, stdout, stderr: '' },
      )
    }
  }

  /** Run a command and return its stdout. Throws when the command fails. */
  async text(args: string[], options?: RunOptions): Promise<string> {
    const argv = ['--json', ...args]
    if (options?.timeoutMs) {
      argv.push('--timeout', String(Math.ceil(options.timeoutMs / 1000)))
    }
    const result = await this.run(argv, {
      ...options,
      // The process has to outlive the request it is waiting on.
      timeoutMs: options?.timeoutMs ? options.timeoutMs + 10_000 : undefined,
    })
    if (result.exitCode !== 0) {
      throw new MoatError(
        `moat ${args.join(' ')} failed (exit ${result.exitCode}): ${firstLine(
          result.stderr || result.stdout,
        )}`,
        {
          command: describeCommand(args),
          exitCode: result.exitCode,
          stdout: result.stdout,
          stderr: result.stderr,
        },
      )
    }
    return result.stdout
  }

  /**
   * Make a request and report its status, whether or not the API accepted it.
   *
   * Use this only where the status is the assertion. Anything else goes
   * through a typed command — and where no command exists, the CLI gains one.
   */
  async raw(
    method: string,
    path: string,
    body?: unknown,
    options?: { headers?: Record<string, string>; timeoutMs?: number },
  ): Promise<RawResponse> {
    const args = ['api', method.toUpperCase(), path]
    if (body !== undefined) {
      args.push('--body', '-')
    }
    for (const [name, value] of Object.entries(options?.headers ?? {})) {
      args.push('--header', `${name}: ${value}`)
    }
    const stdout = await this.text(args, {
      stdin: body === undefined ? undefined : JSON.stringify(body),
      timeoutMs: options?.timeoutMs,
    })
    const parsed = JSON.parse(stdout) as { status: number; body: unknown }
    return { status: parsed.status, body: parsed.body }
  }

  /**
   * Run a long-lived command that prints newline-delimited JSON, handing each
   * object to `onEvent` as it arrives.
   *
   * The returned handle must be stopped: a stream left running outlives the
   * test and holds the worker open.
   */
  stream<T>(
    args: string[],
    onEvent: (event: T) => void,
    onError?: (error: Error) => void,
  ): StreamHandle {
    const child = spawn(MOAT_BIN, ['--json', ...args], {
      env: { ...process.env, ...this.baseEnv },
    })
    let buffer = ''

    child.stdout.on('data', (chunk: Buffer) => {
      buffer += chunk.toString()
      const lines = buffer.split('\n')
      buffer = lines.pop() ?? ''
      for (const line of lines) {
        if (line.trim() === '') continue
        try {
          onEvent(JSON.parse(line) as T)
        } catch {
          // A partial or non-JSON line is not worth failing a test over.
        }
      }
    })
    child.on('error', (error) => onError?.(error))

    return {
      stop: () => {
        child.kill('SIGTERM')
      },
    }
  }

  /**
   * A driver authenticated with an explicit API key.
   *
   * The sandbox token is blanked alongside it: it takes precedence in the CLI,
   * so leaving it in place would silently authenticate as the wrong identity.
   */
  withApiKey(apiKey: string): Moat {
    return new Moat(this.p, {
      ...this.baseEnv,
      USER_AUTH_TOKEN: '',
      MOATLESS_API_KEY: apiKey,
    })
  }

  /**
   * A driver with no credentials, for asserting that a route rejects anonymous
   * callers.
   *
   * Both variables are set to the empty string rather than unset: the child
   * inherits this process's environment, so blanking them is the only way to
   * hide a credential the runner itself carries. The CLI reads an empty value
   * as "no credential", and an empty `MOATLESS_API_KEY` also stops it falling
   * back to the credentials file.
   */
  unauthenticated(): Moat {
    return new Moat(this.p, {
      ...this.baseEnv,
      USER_AUTH_TOKEN: '',
      MOATLESS_API_KEY: '',
    })
  }

  private run(argv: string[], options?: RunOptions): Promise<RunResult> {
    const timeoutMs = options?.timeoutMs ?? DEFAULT_TIMEOUT_MS
    return new Promise<RunResult>((resolve, reject) => {
      const child = spawn(MOAT_BIN, argv, {
        env: { ...process.env, ...this.baseEnv, ...options?.env },
      })
      let stdout = ''
      let stderr = ''
      const timer = setTimeout(() => {
        child.kill('SIGKILL')
        reject(
          new MoatError(`moat ${argv.join(' ')} timed out after ${timeoutMs}ms`, {
            command: describeCommand(argv),
            exitCode: null,
            stdout,
            stderr,
          }),
        )
      }, timeoutMs)

      child.stdout.on('data', (chunk: Buffer) => {
        stdout += chunk.toString()
      })
      child.stderr.on('data', (chunk: Buffer) => {
        stderr += chunk.toString()
      })
      child.on('error', (error) => {
        clearTimeout(timer)
        reject(
          new MoatError(`moat could not be started: ${describe(error)}`, {
            command: describeCommand(argv),
            exitCode: null,
            stdout,
            stderr,
          }),
        )
      })
      child.on('close', (exitCode) => {
        clearTimeout(timer)
        resolve({ stdout, stderr, exitCode })
      })

      if (options?.stdin !== undefined) {
        child.stdin.write(options.stdin)
      }
      child.stdin.end()
    })
  }
}

/**
 * Build the driver for the active profile, minting a credential first when the
 * environment needs one.
 *
 * Under `ambient` there is nothing to do: the sandbox is already logged in.
 * Under `password` the login happens over HTTP once per process — the only
 * request in the suite that does not go through the CLI, because it is what
 * gives the CLI something to authenticate with.
 */
export async function createDriver(p: Profile = profile()): Promise<Moat> {
  if (p.auth === 'ambient') {
    return new Moat(p)
  }
  const apiKey = await mintApiKey(
    p.baseUrl,
    process.env.ADMIN_USERNAME || 'admin',
    process.env.ADMIN_PASSWORD || 'ci-admin-password-1234',
  )
  return new Moat(p, { MOATLESS_API_KEY: apiKey })
}

/**
 * Log in with a password and mint an API key for that identity.
 *
 * Exported so a test that needs a second identity can build a driver for it —
 * only possible where the profile reports the `seedUsers` capability.
 */
export async function mintApiKey(
  baseUrl: string,
  username: string,
  password: string,
): Promise<string> {
  const login = await fetch(`${baseUrl}/api/v1/auth/password/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  })
  if (!login.ok) {
    throw new Error(`Login for '${username}' failed: HTTP ${login.status}: ${await login.text()}`)
  }
  const cookie = login.headers
    .getSetCookie()
    .map((raw) => raw.split(';')[0].trim())
    .find((pair) => pair.startsWith('moatless_session='))
  if (!cookie) {
    throw new Error(`Login for '${username}' set no session cookie`)
  }

  const created = await fetch(`${baseUrl}/api/v1/auth/api-keys`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Cookie: cookie },
    body: JSON.stringify({ name: `e2e-${username}` }),
  })
  if (!created.ok) {
    throw new Error(
      `Minting an API key for '${username}' failed: HTTP ${created.status}: ${await created.text()}`,
    )
  }
  const { key } = (await created.json()) as { key: string }
  return key
}

function describeCommand(args: string[]): string {
  return `moat ${args.join(' ')}`
}

function firstLine(text: string): string {
  return text.trim().split('\n')[0] || '(no output)'
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

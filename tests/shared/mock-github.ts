/**
 * A stand-in GitHub API for the proxy tests.
 *
 * The backend reads `GITHUB_API_BASE_URL` when it starts, so the port is fixed
 * and agreed in advance rather than chosen here.
 *
 * It answers the handful of routes the proxy calls, and records every request
 * so a test can assert on what the backend actually sent — which is the point:
 * the response shapes are trivial, and what matters is the base branch it
 * resolved, the footer it baked into the body, and the path it chose.
 */

import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http'

export const MOCK_PR_NUMBER = 4242
export const MOCK_COMMENT_ID = 987_654
export const MOCK_REPLY_ID = 111_222
export const MOCK_DEFAULT_BRANCH = 'main'
export const MOCK_LOGIN = 'mock-octocat'

/** The port the backend is pointed at, unless `GITHUB_MOCK_PORT` says otherwise. */
export const DEFAULT_MOCK_PORT = 18_092

export interface CapturedRequest {
  method: string
  path: string
  body: Record<string, unknown>
}

interface Answer {
  status: number
  body: unknown
}

function route(method: string, path: string): Answer {
  const segments = path.replace(/^\/+|\/+$/g, '').split('/')
  const [first, owner, repo, ...rest] = segments

  if (method === 'GET' && first === 'user') {
    return { status: 200, body: { login: MOCK_LOGIN, id: 1 } }
  }

  if (first === 'repos') {
    if (method === 'POST' && rest.length === 1 && rest[0] === 'pulls') {
      return {
        status: 201,
        body: {
          number: MOCK_PR_NUMBER,
          html_url: `https://github.com/${owner}/${repo}/pull/${MOCK_PR_NUMBER}`,
          title: 'mock pr',
          state: 'open',
        },
      }
    }
    if (method === 'POST' && rest.length === 3 && rest[0] === 'issues' && rest[2] === 'comments') {
      return { status: 201, body: { id: MOCK_COMMENT_ID } }
    }
    if (
      method === 'POST' &&
      rest.length === 5 &&
      rest[0] === 'pulls' &&
      rest[2] === 'comments' &&
      rest[4] === 'replies'
    ) {
      return { status: 201, body: { id: MOCK_REPLY_ID } }
    }
    if (method === 'GET') {
      // The default-branch lookup, and the PR fetch the hooks make.
      return {
        status: 200,
        body: {
          number: MOCK_PR_NUMBER,
          title: 'mock pr',
          body: '',
          state: 'open',
          default_branch: MOCK_DEFAULT_BRANCH,
          head: { ref: 'feat/mock', sha: 'deadbeef' },
          base: { ref: MOCK_DEFAULT_BRANCH, sha: 'cafebabe' },
        },
      }
    }
  }

  return { status: 404, body: { message: 'mock github: no route' } }
}

export class MockGithub {
  private server: Server | null = null
  private readonly captured: CapturedRequest[] = []

  async start(port = DEFAULT_MOCK_PORT): Promise<void> {
    const server = createServer((req, res) => this.handle(req, res))
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject)
      server.listen(port, () => resolve())
    })
    this.server = server
  }

  async stop(): Promise<void> {
    const server = this.server
    this.server = null
    if (server) {
      await new Promise<void>((resolve) => server.close(() => resolve()))
    }
  }

  /** Every request the backend has made so far. */
  requests(): CapturedRequest[] {
    return [...this.captured]
  }

  /** Wait for a matching request. Returns null if none arrives in time. */
  async waitFor(
    match: (request: CapturedRequest) => boolean,
    timeoutMs = 5000,
  ): Promise<CapturedRequest | null> {
    const deadline = Date.now() + timeoutMs
    for (;;) {
      const found = this.captured.find(match)
      if (found) {
        return found
      }
      if (Date.now() >= deadline) {
        return null
      }
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
  }

  private handle(req: IncomingMessage, res: ServerResponse): void {
    const chunks: Buffer[] = []
    req.on('data', (chunk: Buffer) => chunks.push(chunk))
    req.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf-8')
      let body: Record<string, unknown> = {}
      try {
        body = raw ? (JSON.parse(raw) as Record<string, unknown>) : {}
      } catch {
        // A non-JSON body is not something the proxy sends; record it empty.
      }
      const path = (req.url ?? '/').split('?')[0]!
      this.captured.push({ method: req.method ?? 'GET', path, body })

      const answer = route(req.method ?? 'GET', path)
      res.writeHead(answer.status, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify(answer.body))
    })
  }
}

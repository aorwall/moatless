import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createServer } from 'node:http'
import path from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import { WebSocketServer } from 'ws'

const script = path.resolve(
  import.meta.dirname,
  '../../.agents/skills/rpc-verification/scripts/rpc.mjs',
)
const cleanups = []
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup()
})
async function server(onRequest, onMessage) {
  const http = createServer(
    onRequest ??
      ((_request, response) => {
        response.writeHead(200, { 'content-type': 'application/json' })
        response.end(JSON.stringify({ ticket: 'test-ticket' }))
      }),
  )
  const ws = new WebSocketServer({ server: http })
  ws.on('connection', (socket) =>
    socket.on('message', (raw) => onMessage?.(socket, JSON.parse(raw.toString()))),
  )
  cleanups.push(async () => {
    for (const client of ws.clients) client.terminate()
    await new Promise((resolve) => ws.close(resolve))
    http.closeAllConnections()
    await new Promise((resolve) => http.close(resolve))
  })
  await new Promise((resolve, reject) => {
    http.once('error', reject)
    http.listen(0, '127.0.0.1', resolve)
  })
  return `http://127.0.0.1:${http.address().port}`
}
function run(base, args = ['server.getConfig', '--wait=700'], env = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [script, ...args], {
      env: {
        ...process.env,
        RUNTIME_DIR: path.join(import.meta.dirname, 'no-such-runtime-dir'),
        USER_AUTH_TOKEN: 'test-only',
        MOATLESS_BASE: base,
        ...env,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = '',
      stderr = ''
    child.stdout.on('data', (data) => {
      stdout += data
    })
    child.stderr.on('data', (data) => {
      stderr += data
    })
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      reject(new Error('Probe did not terminate'))
    }, 4000)
    child.on('error', reject)
    child.on('close', (code) => {
      clearTimeout(timer)
      resolve({ code, stdout, stderr })
    })
  })
}
const exit = (value) => ({ _tag: 'Exit', requestId: 1, exit: { _tag: 'Success', value } })

describe('RPC probe process contract', () => {
  it('keeps complete output and parses flags without a payload', async () => {
    let request
    const value = 'payload'.repeat(1000)
    const base = await server(undefined, (socket, frames) => {
      request = frames[0]
      socket.send(JSON.stringify(exit(value)))
    })
    const result = await run(base)
    expect(result.code).toBe(0)
    expect(request.payload).toEqual({})
    expect(JSON.parse(result.stdout)).toEqual([exit(value)])
    expect(result.stderr).toContain('<- Exit')
  })

  it('terminates even when a successful peer ignores the close handshake', async () => {
    const http = createServer((_request, response) => response.end('{"ticket":"test-ticket"}'))
    const sockets = new Set()
    http.on('upgrade', (request, socket) => {
      sockets.add(socket)
      const accept = createHash('sha1')
        .update(`${request.headers['sec-websocket-key']}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
        .digest('base64')
      socket.write(
        `HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`,
      )
      socket.once('data', () => {
        const data = Buffer.from(JSON.stringify(exit('ok')))
        socket.write(Buffer.concat([Buffer.from([0x81, data.length]), data]))
      })
    })
    cleanups.push(async () => {
      for (const socket of sockets) socket.destroy()
      http.closeAllConnections()
      await new Promise((resolve) => http.close(resolve))
    })
    await new Promise((resolve, reject) => {
      http.once('error', reject)
      http.listen(0, '127.0.0.1', resolve)
    })
    const result = await run(`http://127.0.0.1:${http.address().port}`)
    expect(result.code).toBe(0)
    expect(JSON.parse(result.stdout)).toEqual([exit('ok')])
  })

  it.each(['Fail', 'Die'])('fails for Exit/Failure with a %s cause', async (tag) => {
    const frame = {
      _tag: 'Exit',
      requestId: 1,
      exit: { _tag: 'Failure', cause: [{ _tag: tag, error: 'refused' }] },
    }
    const base = await server(undefined, (socket) => socket.send(JSON.stringify(frame)))
    const result = await run(base)
    expect(result.code).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual([frame])
  })

  it('exchanges the sandbox token for the ticket', async () => {
    let token
    const base = await server(
      (request, response) => {
        token = request.headers['x-user-token']
        response.end('{"ticket":"test-ticket"}')
      },
      (socket) => socket.send(JSON.stringify(exit('ok'))),
    )
    const result = await run(base)
    expect(result.code).toBe(0)
    expect(token).toBe('test-only')
  })

  it('fails without a sandbox token', async () => {
    const result = await run('http://127.0.0.1:1', undefined, { USER_AUTH_TOKEN: '' })
    expect(result.code).toBe(1)
    expect(result.stderr).toContain('No sandbox token')
  })

  it('bounds ticket acquisition', async () => {
    const base = await server(() => {})
    const result = await run(base, ['server.getConfig', '--wait=100'])
    expect(result.code).toBe(1)
    expect(result.stderr).toContain('Timed out')
  })

  it('fails an empty stream timeout', async () => {
    const base = await server()
    const result = await run(base, ['sandbox.subscribeStatus', '--stream', '--wait=100'])
    expect(result.code).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual([])
  })

  it('answers Ping, acknowledges chunks, and preserves state transitions', async () => {
    const received = []
    const base = await server(undefined, (socket, frames) => {
      received.push(frames)
      if (Array.isArray(frames))
        socket.send(
          JSON.stringify([{ _tag: 'Ping' }, { _tag: 'Chunk', requestId: 1, values: ['ready'] }]),
        )
      else if (
        frames._tag === 'Ack' &&
        received.filter((frame) => frame._tag === 'Ack').length === 1
      ) {
        socket.send(JSON.stringify({ _tag: 'Chunk', requestId: 1, values: ['stopped'] }))
      }
    })
    const result = await run(base, [
      'sandbox.subscribeStatus',
      '--stream',
      '{"threadId":"t1"}',
      '--wait=300',
    ])
    expect(result.code).toBe(0)
    expect(JSON.parse(result.stdout).map((frame) => frame.values[0])).toEqual(['ready', 'stopped'])
    expect(received[0][0].payload).toEqual({ threadId: 't1' })
    expect(received).toContainEqual({ _tag: 'Pong' })
    expect(received.filter((frame) => frame._tag === 'Ack')).toHaveLength(2)
  })

  it.each(['malformed', 'close', 'empty-success'])('rejects %s responses', async (kind) => {
    const base = await server(undefined, (socket) => {
      if (kind === 'close') socket.close()
      else socket.send(kind === 'malformed' ? 'not-json' : JSON.stringify(exit(null)))
    })
    const result = await run(base, ['sandbox.subscribeStatus', '--stream', '--wait=500'])
    expect(result.code).toBe(1)
  })

  it.each([401, 503])('fails a ticket HTTP %s', async (status) => {
    const base = await server((_request, response) => {
      response.writeHead(status)
      response.end('{}')
    })
    const result = await run(base)
    expect(result.code).toBe(1)
    expect(result.stderr).toContain(`HTTP ${status}`)
  })

  it.each(['--wait=0', '--wait=NaN', '--wait=-1', '--wait=2147483648', '--unknown'])(
    'rejects invalid option %s',
    async (option) => {
      const result = await run('http://127.0.0.1:1', ['server.getConfig', option])
      expect(result.code).toBe(1)
    },
  )
})

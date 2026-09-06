import { pathToFileURL } from 'node:url'

/** Probe one UI-RPC call; stdout contains the complete collected frame array. */
export async function probe(args, env = process.env) {
  const [tag, ...rest] = args
  let payload = {}
  let hasPayload = false
  let stream = false
  let requestedWait
  for (const arg of rest) {
    if (arg === '--stream') stream = true
    else if (arg.startsWith('--wait=')) requestedWait = Number(arg.slice(7))
    else if (!arg.startsWith('--') && !hasPayload) {
      payload = JSON.parse(arg)
      hasPayload = true
    } else throw new Error(`Unexpected argument: ${arg}`)
  }
  if (!tag || tag.startsWith('--')) {
    throw new Error('Usage: rpc.mjs <method> [payloadJson] [--stream] [--wait=ms]')
  }
  const waitMs = requestedWait ?? (stream ? 8000 : 5000)
  if (!Number.isSafeInteger(waitMs) || waitMs <= 0 || waitMs > 2_147_483_647) {
    throw new Error('--wait must be a positive integer within the Node timer range')
  }
  if (!env.MOATLESS_API_KEY) throw new Error('MOATLESS_API_KEY is not set')
  const base = new URL(env.MOATLESS_BASE ?? 'http://localhost:8080')
  if (!['http:', 'https:'].includes(base.protocol))
    throw new Error('MOATLESS_BASE must use HTTP(S)')
  const frames = []
  let socket
  let timer
  const controller = new AbortController()
  let chunks = 0
  let finish
  const completed = new Promise((resolve) => {
    finish = resolve
  })
  let settled = false
  const settle = (error) => {
    if (settled) return
    settled = true
    clearTimeout(timer)
    controller.abort()
    if (socket && socket.readyState < WebSocket.CLOSING) socket.close()
    finish({ frames, error })
  }
  timer = setTimeout(() => {
    settle(
      stream && chunks > 0 ? undefined : new Error('Timed out without a successful RPC response'),
    )
  }, waitMs)
  try {
    const response = await fetch(new URL('/api/auth/websocket-ticket', base), {
      method: 'POST',
      headers: { authorization: `Bearer ${env.MOATLESS_API_KEY}` },
      signal: controller.signal,
    })
    if (!response.ok) throw new Error(`Ticket request failed: HTTP ${response.status}`)
    const { ticket } = await response.json()
    if (typeof ticket !== 'string' || !ticket) throw new Error('Ticket response has no ticket')
    if (settled) return await completed
    const url = new URL('/ws', base)
    url.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:'
    url.searchParams.set('wsTicket', ticket)
    socket = new WebSocket(url)
    socket.addEventListener('open', () => {
      socket.send(JSON.stringify([{ _tag: 'Request', id: 1, tag, payload, headers: [] }]))
    })
    socket.addEventListener('message', (event) => {
      if (settled) return
      try {
        const received = JSON.parse(event.data)
        for (const frame of Array.isArray(received) ? received : [received]) {
          if (!frame || typeof frame._tag !== 'string') throw new Error('Malformed RPC frame')
          if (frame._tag === 'Ping') {
            socket.send(JSON.stringify({ _tag: 'Pong' }))
            continue
          }
          if (!['Chunk', 'Exit'].includes(frame._tag) || String(frame.requestId) !== '1') {
            throw new Error('Unexpected RPC frame or request ID')
          }
          frames.push(frame)
          console.error(`<- ${frame._tag} ${Buffer.byteLength(JSON.stringify(frame))}B`)
          if (frame._tag === 'Chunk') {
            if (!Array.isArray(frame.values)) throw new Error('Malformed Chunk values')
            chunks += 1
            socket.send(JSON.stringify({ _tag: 'Ack', requestId: frame.requestId }))
          } else if (frame.exit?._tag === 'Failure') {
            settle(new Error('RPC returned Exit/Failure; inspect stdout for its cause'))
            break
          } else if (frame.exit?._tag === 'Success') {
            settle(stream && chunks === 0 ? new Error('Stream ended without a Chunk') : undefined)
            break
          } else throw new Error('Malformed Exit value')
        }
      } catch (error) {
        settle(error)
      }
    })
    socket.addEventListener('error', () => settle(new Error('WebSocket transport error')))
    socket.addEventListener('close', () =>
      settle(new Error('WebSocket closed before RPC completion')),
    )
  } catch (error) {
    settle(error)
  }
  return await completed
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const { frames, error } = await probe(process.argv.slice(2))
    if (error) console.error(error.message)
    // A peer that ignores the close handshake must not extend the CLI deadline.
    process.stdout.write(`${JSON.stringify(frames, null, 2)}\n`, () => process.exit(error ? 1 : 0))
  } catch (error) {
    console.error(error.message)
    process.exitCode = 1
  }
}

/** Serve a local Vite origin over TLS, including WebSocket upgrades. */

import * as fs from 'node:fs'
import * as net from 'node:net'
import * as tls from 'node:tls'

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`)
  return i === -1 ? fallback : process.argv[i + 1]
}

const listenPort = Number(arg('listen', 8444))
const targetPort = Number(arg('target', 5733))
const targetHost = arg('target-host', '127.0.0.1')
const cert = fs.readFileSync(arg('cert', '/tmp/pw/cert.pem'))
const key = fs.readFileSync(arg('key', '/tmp/pw/key.pem'))

const server = tls.createServer({ cert, key }, (client) => {
  const upstream = net.connect(targetPort, targetHost)
  // Either half closing or erroring tears down the pair; without this a failed
  // upstream connect leaves the browser waiting on a socket nobody will answer.
  const end = () => {
    client.destroy()
    upstream.destroy()
  }
  client.on('error', end)
  client.on('close', end)
  upstream.on('error', end)
  upstream.on('close', end)
  client.pipe(upstream)
  upstream.pipe(client)
})

server.on('error', (error) => {
  console.error(`tls-front failed: ${error.message}`)
  process.exit(1)
})

server.listen(listenPort, '127.0.0.1', () => {
  console.log(`tls-front: https://127.0.0.1:${listenPort} -> http://${targetHost}:${targetPort}`)
})

import * as net from 'node:net'

// Port range constants
export const PORT_RANGES = {
  PREVIEW: { min: 3000, max: 3999 },
  WORKER: { min: 4000, max: 4999 },
  GENERAL: { min: 8000, max: 8999 },
} as const

export type PortType = 'preview' | 'worker' | 'general'

/**
 * Checks if a port is available by attempting to connect to it
 */
async function isPortAvailable(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const client = new net.Socket()
    client.setTimeout(100)

    client.on('connect', () => {
      client.destroy()
      resolve(false) // Port is in use
    })

    client.on('timeout', () => {
      client.destroy()
      resolve(true) // Port is available
    })

    client.on('error', () => {
      resolve(true) // Port is available
    })

    client.connect(port, '127.0.0.1')
  })
}

/**
 * Finds the first available port in the specified range
 */
export async function findAvailablePort(type: PortType = 'general'): Promise<number> {
  const range = PORT_RANGES[type.toUpperCase() as keyof typeof PORT_RANGES]

  if (!range) {
    throw new Error(`Invalid port type: ${type}`)
  }

  for (let port = range.min; port <= range.max; port++) {
    if (await isPortAvailable(port)) {
      return port
    }
  }

  throw new Error(`No available ports in range ${range.min}-${range.max} for type: ${type}`)
}

/**
 * Finds any available port using the OS
 */
export async function findAnyAvailablePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address() as net.AddressInfo
      const port = address.port
      server.close(() => resolve(port))
    })
    server.on('error', reject)
  })
}

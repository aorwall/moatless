/**
 * File Explorer Server — Bun fullstack entry point.
 *
 * Serves:
 *   - "/" → React SPA (Bun auto-bundles TSX, CSS, Tailwind via HTML import)
 *   - "/api/*" → File API endpoints (direct filesystem access)
 *
 * Runs as an independent server inside the sandbox container on port 4000.
 */
import { FileService } from './services/file.service'

const isProduction = process.env['NODE_ENV'] === 'production'

// In development, Bun auto-bundles the HTML import at serve-time.
// In production, we serve pre-built static files from dist/public/.
// biome-ignore lint/suspicious/noExplicitAny: Bun HTML import returns an opaque type
let homepage: any
if (!isProduction) {
  // Bun HTML import — bundled at serve-time in development
  homepage = (await import('./public/index.html')).default
}

const WORKSPACE_PATH = process.env['WORKSPACE_PATH'] || '/opt/moatless/repo'
const PORT = Number(process.env['PORT']) || 4000

const fileService = new FileService(WORKSPACE_PATH)

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function errorResponse(error: unknown, status = 500): Response {
  const message = error instanceof Error ? error.message : String(error)
  return jsonResponse({ error: message }, status)
}

async function handleTree(req: Request | BunRequest): Promise<Response> {
  try {
    const url = new URL(req.url)
    const rootPath = url.searchParams.get('path') || '.'
    const result = await fileService.getFileTreeWithChanges(rootPath)
    return jsonResponse(result)
  } catch (error) {
    return errorResponse(error)
  }
}

async function handleReadFile(req: Request | BunRequest): Promise<Response> {
  try {
    const url = new URL(req.url)
    const filePath = url.searchParams.get('path')
    if (!filePath) return jsonResponse({ error: 'path query parameter is required' }, 400)

    const withChanges = url.searchParams.get('changes') === 'true'
    if (withChanges) {
      const result = await fileService.readFileWithChanges(filePath)
      return jsonResponse(result)
    }

    const content = await fileService.readFile(filePath)
    return jsonResponse({ content })
  } catch (error) {
    return errorResponse(error)
  }
}

async function handleWriteFile(req: Request | BunRequest): Promise<Response> {
  try {
    const body = (await req.json()) as { filePath?: string; content?: string }
    if (!body.filePath) return jsonResponse({ error: 'filePath is required' }, 400)
    if (body.content === undefined) return jsonResponse({ error: 'content is required' }, 400)

    await fileService.writeFile(body.filePath, body.content)
    return jsonResponse({ success: true })
  } catch (error) {
    return errorResponse(error)
  }
}

async function handleListFiles(req: Request | BunRequest): Promise<Response> {
  try {
    const url = new URL(req.url)
    const relativePath = url.searchParams.get('path') || '.'
    const recursive = url.searchParams.get('recursive') === 'true'
    const files = await fileService.listFiles(relativePath, recursive)
    return jsonResponse({ files })
  } catch (error) {
    return errorResponse(error)
  }
}

async function handleSearch(req: Request | BunRequest): Promise<Response> {
  try {
    const body = (await req.json()) as {
      query?: string
      caseSensitive?: boolean
      maxResults?: number
    }
    if (!body.query) return jsonResponse({ error: 'query is required' }, 400)

    const result = await fileService.searchFiles({
      query: body.query,
      caseSensitive: body.caseSensitive,
      maxResults: body.maxResults,
    })
    return jsonResponse(result)
  } catch (error) {
    return errorResponse(error)
  }
}

function serveStaticFile(filePath: string): Response | null {
  const file = Bun.file(filePath)
  // Bun.file doesn't throw for missing files; check size synchronously
  if (file.size === 0) return null
  return new Response(file)
}

if (isProduction) {
  const distPublic = new URL('./public/', import.meta.url).pathname

  Bun.serve({
    port: PORT,
    routes: {
      '/': () => new Response(Bun.file(`${distPublic}index.html`), {
        headers: { 'Content-Type': 'text/html' },
      }),
      '/api/tree': { GET: handleTree },
      '/api/file': { GET: handleReadFile, POST: handleWriteFile },
      '/api/files': { GET: handleListFiles },
      '/api/search': { POST: handleSearch },
    },
    fetch(req) {
      const url = new URL(req.url)
      // Try serving a static asset from dist/public/
      const asset = serveStaticFile(`${distPublic}${url.pathname.slice(1)}`)
      if (asset) return asset
      // SPA fallback
      return new Response(Bun.file(`${distPublic}index.html`), {
        headers: { 'Content-Type': 'text/html' },
      })
    },
    development: false,
  })
} else {
  Bun.serve({
    port: PORT,
    routes: {
      '/': homepage,
      '/api/tree': { GET: handleTree },
      '/api/file': { GET: handleReadFile, POST: handleWriteFile },
      '/api/files': { GET: handleListFiles },
      '/api/search': { POST: handleSearch },
    },
    fetch(_req) {
      // SPA fallback: serve index.html for all non-API routes
      return new Response(Bun.file(new URL('./public/index.html', import.meta.url)))
    },
    development: true,
  })
}

console.log(`File Explorer server running on http://localhost:${PORT}`)
console.log(`Workspace: ${WORKSPACE_PATH}`)

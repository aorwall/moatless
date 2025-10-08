# Configuration Management Implementation Plan

This plan provides step-by-step instructions for implementing type-safe configuration management.

## Step 1: Create Config Schema

Create `apps/backend/src/config/index.ts`:

```typescript
import { z } from 'zod'

// Define schema with all environment variables
const configSchema = z.object({
  // Server
  nodeEnv: z.enum(['development', 'production', 'test']).default('development'),
  port: z.coerce.number().default(8000),
  host: z.string().default('0.0.0.0'),

  // Storage
  storagePath: z.string().default('./data'),
  cacheEnabled: z.coerce.boolean().default(false),

  // CORS
  corsOrigin: z.string().default('http://localhost:3000'),

  // GitHub OAuth (optional)
  githubOAuthEnabled: z.coerce.boolean().default(false),
  githubClientId: z.string().optional(),
  githubClientSecret: z.string().optional(),
  githubCallbackUrl: z.string().optional(),

  // Session (optional)
  sessionSecret: z.string().optional(),
  sessionMaxAge: z.coerce.number().default(604800),

  // URLs
  frontendUrl: z.string().default('http://localhost:3000'),
  apiUrl: z.string().default('http://localhost:8000'),

  // Logging
  logLevel: z.enum(['trace', 'debug', 'info', 'warn', 'error', 'fatal']).default('info'),
  useJsonLogging: z.coerce.boolean().default(false),

  // Proxy
  useHostnameRouting: z.coerce.boolean().default(false),
  proxyDomain: z.string().optional(),
  proxyPort: z.coerce.number().default(80),
  proxyProtocol: z.enum(['http', 'https']).default('http'),

  // Worker Mode
  workerMode: z.coerce.boolean().default(false),
  workerApiUrl: z.string().optional(),
  workerApiKey: z.string().optional(),
  workspaceDir: z.string().optional(),
})

// Infer TypeScript type from schema
export type Config = z.infer<typeof configSchema>

// Parse and validate environment variables
function loadConfig(): Config {
  const rawConfig = {
    nodeEnv: process.env['NODE_ENV'],
    port: process.env['PORT'],
    host: process.env['HOST'],
    storagePath: process.env['STORAGE_PATH'],
    cacheEnabled: process.env['CACHE_ENABLED'],
    corsOrigin: process.env['CORS_ORIGIN'],
    githubOAuthEnabled: process.env['GITHUB_OAUTH_ENABLED'],
    githubClientId: process.env['GITHUB_CLIENT_ID'],
    githubClientSecret: process.env['GITHUB_CLIENT_SECRET'],
    githubCallbackUrl: process.env['GITHUB_CALLBACK_URL'],
    sessionSecret: process.env['SESSION_SECRET'],
    sessionMaxAge: process.env['SESSION_MAX_AGE'],
    frontendUrl: process.env['FRONTEND_URL'],
    apiUrl: process.env['API_URL'],
    logLevel: process.env['LOG_LEVEL'],
    useJsonLogging: process.env['USE_JSON_LOGGING'],
    useHostnameRouting: process.env['USE_HOSTNAME_ROUTING'],
    proxyDomain: process.env['PROXY_DOMAIN'],
    proxyPort: process.env['PROXY_PORT'],
    proxyProtocol: process.env['PROXY_PROTOCOL'],
    workerMode: process.env['WORKER_MODE'],
    workerApiUrl: process.env['WORKER_API_URL'],
    workerApiKey: process.env['WORKER_API_KEY'],
    workspaceDir: process.env['WORKSPACE_DIR'],
  }

  const result = configSchema.safeParse(rawConfig)

  if (!result.success) {
    console.error('❌ Invalid configuration:')
    console.error(result.error.format())
    throw new Error('Configuration validation failed')
  }

  return result.data
}

// Export singleton config instance
export const config = loadConfig()

// Helper to check if running in production
export const isProduction = config.nodeEnv === 'production'

// Helper to check if running in test
export const isTest = config.nodeEnv === 'test'

// Helper to check if running in development
export const isDevelopment = config.nodeEnv === 'development'
```

## Step 2: Add Conditional Validation (Optional)

For config that's required in certain modes:

```typescript
const configSchema = z.object({
  // ... other config
  githubOAuthEnabled: z.coerce.boolean().default(false),
  githubClientId: z.string().optional(),
  githubClientSecret: z.string().optional(),
}).refine(
  (data) => {
    // If OAuth is enabled, client ID and secret are required
    if (data.githubOAuthEnabled) {
      return !!data.githubClientId && !!data.githubClientSecret
    }
    return true
  },
  {
    message: 'When GITHUB_OAUTH_ENABLED=true, GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET are required',
  }
)
```

## Step 3: Environment-Specific Defaults (Optional)

```typescript
const configSchema = z.object({
  logLevel: z.enum(['trace', 'debug', 'info', 'warn', 'error']).default(
    process.env['NODE_ENV'] === 'production' ? 'info' : 'debug'
  ),

  corsOrigin: z.string().default(
    process.env['NODE_ENV'] === 'production'
      ? 'https://app.example.com'
      : 'http://localhost:3000'
  ),
})
```

## Step 4: Update Application Bootstrap

```typescript
// apps/backend/src/index.ts
import { config } from './config/index.js'

const server = serve({
  fetch: app.fetch,
  port: config.port,
  hostname: config.host,
})

console.log(`Server listening on http://${config.host}:${config.port}`)
```

## Step 5: Update Services

```typescript
// apps/backend/src/services/storage.ts
import { config } from '../config/index.js'

export class StorageService {
  private basePath: string

  constructor() {
    this.basePath = config.storagePath
  }
}
```

## Step 6: Update Middleware

```typescript
// apps/backend/src/middleware/cors.ts
import { config } from '../config/index.js'

app.use('*', cors({
  origin: config.corsOrigin,
  credentials: true,
}))
```

## Migration Guide

To migrate existing `process.env` usage:

1. Identify the environment variable: `process.env['SOME_VAR']`
2. Add to config schema with appropriate type
3. Replace usage with `config.someVar`
4. Update `.env.example`

## Testing Configuration

```typescript
// In tests, mock the config
vi.mock('../config', () => ({
  config: {
    port: 8001,
    nodeEnv: 'test',
    // ... other test values
  }
}))
```

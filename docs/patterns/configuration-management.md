# Configuration Management Pattern

## Problem

Configuration scattered across the codebase creates maintenance issues:
- `process.env['PORT']` used directly in 15+ files
- No validation of required environment variables
- Type coercion errors (strings vs numbers vs booleans)
- Hard to track what configuration exists

## Solution

Centralized, type-safe configuration with Zod validation.

**📋 Implementation Plan**: See [Configuration Management Implementation](../plans/configuration-management-implementation.md)

## Rules

### RULE 1: Never Access process.env Directly

```typescript
// ❌ NEVER DO THIS
const port = process.env['PORT']
const isEnabled = process.env['FEATURE_ENABLED'] === 'true'

// ✅ ALWAYS DO THIS
import { config } from '@/config'
const port = config.port
const isEnabled = config.featureEnabled
```

### RULE 2: Import Config from Central Location

```typescript
// ✅ CORRECT - Import and use config
import { config } from '@/config'

const port = config.port
const isOAuthEnabled = config.githubOAuthEnabled

if (config.nodeEnv === 'production') {
  // Production-specific code
}
```

### RULE 3: Add New Config to Schema First

When you need a new environment variable:

1. Add to `configSchema` in `config/index.ts`
2. Add to `.env.example` with description
3. Use via `config.yourNewSetting`

### RULE 4: Use Correct Type Coercion

```typescript
// For booleans
someBoolean: z.coerce.boolean().default(false)

// For numbers
someNumber: z.coerce.number().default(0)

// For enums
someEnum: z.enum(['option1', 'option2']).default('option1')

// For optional strings
someString: z.string().optional()

// For required strings with default
someString: z.string().default('default-value')
```

### RULE 5: Validate Complex Requirements

If a config value is required when another is enabled:

```typescript
const configSchema = z.object({
  featureEnabled: z.coerce.boolean().default(false),
  featureApiKey: z.string().optional(),
}).refine(
  (data) => !data.featureEnabled || !!data.featureApiKey,
  { message: 'featureApiKey is required when featureEnabled is true' }
)
```

## Usage Examples

### In Application Bootstrap
```typescript
import { config } from './config/index'

const server = serve({
  port: config.port,
  hostname: config.host,
})
```

### In Services
```typescript
import { config } from '../config/index'

export class StorageService {
  private basePath = config.storagePath
}
```

### In Middleware
```typescript
import { config } from '../config/index'

app.use('*', cors({
  origin: config.corsOrigin,
  credentials: true,
}))
```

## Testing

```typescript
// Mock config in tests
vi.mock('../config', () => ({
  config: {
    port: 8001,
    nodeEnv: 'test',
  }
}))
```

## Quick Reference

| DO | DON'T |
|-----|-------|
| `import { config }` | `process.env['VAR']` |
| `config.port` | `Number(process.env['PORT'])` |
| Add to schema first | Use env var directly |
| Use `.coerce` for types | Manual string conversion |
| Validate on startup | Fail at runtime |

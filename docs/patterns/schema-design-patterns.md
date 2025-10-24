# Schema Design Patterns

## Core Principles
- **Schema-first**: Define Zod schemas, derive TypeScript types
- **Single source of truth**: One schema per domain concept
- **No circular references**: Prevent serialization failures
- **Runtime validation**: Leverage Zod for data integrity

## Type Modeling Rules
- MUST declare object shapes with `interface` when annotating derived DTOs (`interface TaskPayload`) instead of `type TaskPayload = { ... }`.
- MUST expose readonly collection fields as `readonly T[]` for simple element types and prefer `ReadonlyArray<T>` only for complex generics.
- MUST add optionality with `?` (e.g. `description?: string`) instead of embedding `| undefined` in aliases. Handle null/undefined at the boundary that produces them.
- MUST use tuple syntax `[string, number]` instead of container interfaces when returning fixed-length pairs.
- MUST use primitive types (`string`, `number`, `boolean`, `object`) over wrapper types (`String`, `Number`, `Boolean`, `Object`) in all schema-adjacent code.

```typescript
// CORRECT – interface for DTO and tuple return
interface TaskDraft {
  id: string
  readonly labels: readonly string[]
  metadata?: TaskMetadata
}

export function toTaskDraft(input: z.infer<typeof TaskSchema>): TaskDraft {
  return {
    id: input.id,
    labels: [...input.labels],
    metadata: input.metadata,
  }
}

export function splitTitle(title: string): [string, string] {
  const [prefix = '', ...rest] = title.split(':')
  return [prefix, rest.join(':')]
}

// WRONG – type alias object + wrapper types + |undefined union
type TaskDraftAlias = {
  id: String
  labels: ReadonlyArray<string>
  metadata: TaskMetadata | undefined
}
```

## Status Fields: String Literals Only
```typescript
// ✅ CORRECT - Use string literal unions
status: z.enum(['open', 'running', 'error', 'merged', 'closed'])

// ❌ WRONG - Never use TypeScript enums
enum TaskStatus { OPEN, RUNNING, ERROR }
status: z.nativeEnum(TaskStatus)
```

**Rationale**: String literals are serialization-friendly, more readable in logs/APIs, and avoid TypeScript enum pitfalls.

## Preventing Circular References
```typescript
// ❌ BAD - Creates circular reference
const TaskIterationSchema = z.object({
  task: z.lazy(() => TaskSchema), // Task → iterations → task
  taskId: z.string(),
})

// ✅ GOOD - Use ID references
const TaskIterationSchema = z.object({
  // No task field - access via parent context
  // No taskId field - redundant when nested in task.iterations[]
})
```

## Schema Organization
```
schemas/
├── api.schemas.ts      # Request/response schemas
├── task.schemas.ts     # Domain model schemas
└── index.ts           # Public exports

types/
├── api.ts             # z.infer<typeof Schema>
└── index.ts           # Re-exports
```

## Common Patterns

### Timestamps
```typescript
createdAt: z.date(),
updatedAt: z.date().optional(),
```

### IDs
```typescript
id: z.string().uuid(),
// or for flexibility:
id: z.string(),
```

### Optional vs Required
```typescript
// Required with default
limit: z.number().default(10),

// Truly optional
description: z.string().optional(),
```

### Nested Objects
```typescript
// Inline for simple cases
metadata: z.object({
  key: z.string(),
  value: z.string(),
}),

// Extract for reuse
const MetadataSchema = z.object({...})
metadata: MetadataSchema,
```

### Arrays
```typescript
items: z.array(ItemSchema),
tags: z.array(z.string()).default([]),
```

### Discriminated Unions
```typescript
const EventSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('created'), data: CreatedDataSchema }),
  z.object({ type: z.literal('updated'), data: UpdatedDataSchema }),
])
```

### Transform for Parsing
```typescript
// Parse dates from strings
dateSchema: z.string().datetime().transform(str => new Date(str))

// Coerce types
recursive: z.coerce.boolean().default(false)
```

## Response Schemas
```typescript
// List responses
export const TaskListResponseSchema = z.object({
  items: z.array(TaskSchema),
  pagination: z.object({
    limit: z.number(),
    offset: z.number(),
    total: z.number().optional(),
  }),
})

// Error responses
export const ErrorResponseSchema = z.object({
  error: z.string(),
  details: z.record(z.unknown()).optional(),
})
```

## Schema Evolution
1. **Add optional fields** for backward compatibility
2. **Never remove required fields** in minor versions
3. **Use transforms** to handle legacy data formats
4. **Document deprecations** in schema comments

## Testing Schemas
```typescript
// Always test parse/transform behavior
const result = TaskSchema.safeParse(data)
if (!result.success) {
  console.error(result.error.issues)
}
```

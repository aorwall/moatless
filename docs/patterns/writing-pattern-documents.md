# Writing Pattern Documents for AI Agents

## Purpose

Pattern documents serve as operating instructions for AI agents working in this codebase. Unlike human documentation, these patterns must be precise, unambiguous, and immediately actionable for AI consumption.

## Document Structure

### Title Convention

```markdown
# [Pattern Name]
```

- MUST use singular nouns (e.g., "API Specification", not "API Specifications")
- MUST be descriptive and specific
- MUST avoid generic terms like "Guidelines" or "Best Practices"

### Section Hierarchy

```markdown
# Pattern Name

## Primary Concept
### Specific Implementation

## Another Primary Concept
### Another Implementation
```

- MUST use ## for major concepts
- MUST use ### for specific implementations
- MUST NOT exceed 4 levels of nesting
- Each section MUST contain actionable content

## Content Principles for AI Consumption

### Imperative Directives

Use explicit directive language that AI agents can parse as rules:

```markdown
- MUST use @RestController annotation
- MUST NOT include circular dependencies
- SHOULD provide default values for optional parameters
- CAN use either approach depending on context
- NEVER create files in the root directory
```

**Directive Hierarchy:**
- `MUST` / `MUST NOT` - Hard requirements, violations indicate errors
- `SHOULD` / `SHOULD NOT` - Strong recommendations, exceptions require justification
- `CAN` / `MAY` - Optional approaches, context-dependent
- `NEVER` / `ALWAYS` - Absolute rules with no exceptions

### Code Examples Requirements

Every directive MUST be accompanied by concrete code examples:

```markdown
### API Route Handlers

- MUST use HTTP method-specific Hono methods matching REST semantics

```typescript
// CORRECT
app.post('/api/v1/customers', async (c) => {
  const data = await c.req.json();
  const customer = await createCustomer(data);
  return c.json(customer, 201);
});

app.get('/api/v1/customers/:id', async (c) => {
  const id = c.req.param('id');
  const customer = await getCustomerById(id);
  return c.json(customer);
});

// WRONG
app.post('/api/v1/customers', async (c) => {
  const data = await c.req.json();
  const customer = await saveCustomer(data); // Ambiguous verb
  return c.json(customer);
});

app.get('/api/v1/customers/:id', async (c) => {
  const id = c.req.param('id');
  const customer = await retrieveCustomer(id); // Non-standard verb
  return c.json(customer);
});
```
```

**Code Example Requirements:**
- MUST be complete and compilable
- MUST show both correct and incorrect patterns when applicable
- MUST use realistic variable names from the domain
- MUST include necessary imports and annotations
- SHOULD use actual class names from the codebase

### Documentation Guidance

- SHOULD reference JSDoc only when it adds information beyond what TypeScript types already convey (e.g. documenting units, side effects, or invariants).
- MUST show the JSDoc block before decorators to match ts.dev style.
- MUST NOT encourage `@ts-ignore` or other comment-based suppression unless paired with an explicit rationale.

### Correct/Wrong Pattern Examples

Always provide contrasting examples to prevent misinterpretation:

```markdown
```typescript
// CORRECT - OpenAPI tag definition
const app = new Hono()
  .openapi(
    createRoute({
      method: 'get',
      path: '/api/v1/customers',
      tags: ['Customers'],
      description: 'APIs for managing customers',
    }),
    handler
  );

// WRONG - Overly verbose tag name
const app = new Hono()
  .openapi(
    createRoute({
      method: 'get',
      path: '/api/v1/customers',
      tags: ['Customer Management'], // Too verbose
      description: 'Customer operations',
    }),
    handler
  );
```
```

- MUST label examples as CORRECT/WRONG or GOOD/BAD
- MUST explain why the wrong example is incorrect
- SHOULD include multiple variations when patterns can be misunderstood

## Avoiding AI Confusion

### Specificity Over Generality

```markdown
// TOO VAGUE - AI might misinterpret
- Use appropriate HTTP status codes

// PRECISE - Clear for AI parsing
- MUST return c.json(data, 201) for POST operations creating resources
- MUST return c.json(data) for GET, PUT, PATCH operations (defaults to 200)
- MUST return c.body(null, 204) for DELETE operations
- MUST return c.json(error, 404) when resource not found
```

### Explicit Context

```markdown
### Route Handler Function Naming

When implementing API route handlers:
- MUST use function names matching HTTP semantics:
  - POST → `createResource`
  - GET (single) → `getResourceById`
  - GET (list) → `listResources`
  - PUT → `updateResource`
  - PATCH → `patchResource`
  - DELETE → `deleteResource`
```

- MUST state when patterns apply ("When creating...", "For service methods...")
- MUST specify scope ("In controller classes...", "For JPA entities...")
- MUST NOT assume AI will infer context

### Decision Trees for Complex Scenarios

```markdown
### Choosing Validation Schemas

When implementing API endpoints:

1. For POST operations:
   - MUST use Zod schema for request validation
   - MUST validate with createResourceSchema
   - MUST return validated type from handler

2. For PUT operations:
   - MUST use updateResourceSchema for full updates
   - MUST require all fields in schema
   - MUST return updated resource type

3. For PATCH operations:
   - MUST use partial schema (.partial() in Zod)
   - MUST handle undefined fields (no change)
   - MUST return updated resource type

Example:
```typescript
// POST - Create schema
const createTaskSchema = z.object({
  title: z.string().min(1),
  description: z.string(),
  repositoryId: z.string(),
});

// PATCH - Partial schema
const patchTaskSchema = createTaskSchema.partial();
```
```

## Cross-Reference Strategy

### Pattern Dependencies

```markdown
### Before Reading This Pattern

1. Read [API Best Practices](api-best-practices.md) for route handler patterns
2. Review [Schema Design Patterns](schema-design-patterns.md) for Zod validation
3. Understand [React Query Patterns](react-query.md) for frontend data fetching
```

- MUST list prerequisite patterns at the beginning
- MUST use relative paths for internal links
- MUST maintain dependency order (architectural → implementation → testing)

### Pattern References

```markdown
- See [Error Handling Pattern](error-handling.md) for error response patterns
- Cross-reference with [Dependency Injection Pattern](dependency-injection.md) for service setup
- Review [Cache Invalidation Pattern](cache-invalidation.md) for data synchronization
```

- MUST reference related patterns within content
- MUST be specific about which section to reference
- MUST NOT create circular dependencies

## Executable Elements

### Command Examples

```markdown
## Running Tests

```bash
# Run specific test file
bun test src/features/tasks/__tests__/task-viewer.test.tsx

# Run unit tests only (fast)
bun run test:unit

# Run integration tests
bun run test:integration

# Run all tests with coverage
bun run test:coverage
```
```

- MUST provide exact commands that work in the project
- MUST include command descriptions
- SHOULD show expected output when relevant

### File Paths and Locations

```markdown
apps/frontend/src/features/tasks/
├── api/           # API client functions
├── components/    # React components
├── hooks/         # Custom React hooks
├── stores/        # State management
└── utils/         # Utility functions
```

- MUST use exact file paths from the project
- MUST show directory structure when relevant
- MUST be consistent with actual codebase structure

## Validation Patterns

### Checklist Format

```markdown
## Implementation Checklist

Before committing route handler changes:
1. Verify Zod schema validation is implemented
2. Check handler returns correct HTTP status codes
3. Confirm proper error handling with try/catch
4. Write unit tests for handler logic
5. Run `bun run test:unit` to ensure no regressions
6. Run `bun run type:check` to verify TypeScript types
7. Run `bun run biome:check` to validate formatting
```

- MUST provide step-by-step validation
- MUST include testing requirements
- MUST reference specific commands or tools

## Maintenance Guidelines

### Pattern Evolution

```markdown
## Pattern Updates

When modifying this pattern:
1. Update all code examples to match changes
2. Verify cross-referenced patterns remain consistent
3. Test example code compiles and runs
4. Update version information if applicable
```

### README Integration Guidelines

### Pattern Description Format for README.md

Each pattern entry in the main README.md MUST follow this task-oriented format:

```markdown
[Pattern Name](path) - Description | **File patterns affected**
```

**Examples:**
```markdown
[API Best Practices](api-best-practices.md) - Hono routes, HTTP methods, validation, error handling | **`apps/backend/src/routes/**/*.ts`**
[React Query Patterns](react-query.md) - Data fetching, mutations, cache management | **`apps/frontend/src/features/**/api/*.ts`, `apps/frontend/src/features/**/hooks/*.ts`**
[Schema Design Patterns](schema-design-patterns.md) - Zod schemas, validation, type inference | **`packages/types/src/schemas/**/*.ts`**
```

### Task-Based Organization Requirements

When adding patterns to README.md, organize by:

1. **Foundational Patterns** - Apply to all/most modifications
   - Code Conventions (always required)
   - TypeScript Best Practices (always required)
   - Schema Design Patterns (required for data validation)

2. **Task-Based Sections** - Group by what AI agents are actually doing
   - "Creating or Modifying API Endpoints"
   - "Implementing React Components"
   - "Managing State with React Query"
   - "Writing Tests (Unit, Integration, E2E)"

3. **File Pattern Matching** - Help AI agents identify relevance
   ```markdown
   **File patterns:** `apps/backend/src/routes/**/*.ts`, `apps/frontend/src/features/**/components/*.tsx`
   ```

### Quick Reference Mapping Table

MUST include a task-to-pattern mapping table for mechanical pattern selection:

```markdown
| Task | File Types | Core Patterns | Additional Patterns |
|------|------------|---------------|---------------------|
| Add API endpoint | `apps/backend/src/routes/**/*.ts` | API Best Practices, Schema Design | Error Handling, Dependency Injection |
| Create React component | `apps/frontend/src/features/**/components/*.tsx` | React Query Patterns, Derived State | Error Handling, Testing Patterns |
| Implement data fetching | `apps/frontend/src/features/**/hooks/*.ts` | React Query Patterns, Cache Invalidation | Error Handling |
| Write tests | `**/__tests__/**/*.test.ts(x)` | Frontend Testing, Backend Testing, Strategic Mocking | - |
```

### README Entry Guidelines

- **MUST** specify file patterns affected (e.g., `apps/backend/src/routes/**/*.ts`, `apps/frontend/src/features/**/components/*.tsx`)
- **MUST** list patterns in dependency order (architectural → implementation → testing)
- **MUST** distinguish between required and optional patterns
- **SHOULD** use concrete task verbs (Creating, Implementing, Adding, Writing)
- **SHOULD** include key concepts/technologies for quick scanning (e.g., Hono, React Query, Zod)

### Cross-Reference Strategy

```markdown
### Creating or Modifying API Endpoints
**Required patterns:**
1. [API Best Practices](api-best-practices.md) - Route handlers, HTTP methods, response patterns
2. [Schema Design Patterns](schema-design-patterns.md) - Zod validation, type inference
3. [Dependency Injection Pattern](dependency-injection.md) - Service container setup
4. [Error Handling Pattern](error-handling.md) - Error responses, status codes
```

- MUST list patterns in execution order
- MUST specify which aspect each pattern covers
- MUST use numbered lists for required sequence
- CAN use bullets for context-dependent patterns

## Integration with CLAUDE.md

Pattern documents MUST integrate with the main CLAUDE.md navigation:

```markdown
### 2. Implementing API endpoints or backend services
1. Apply [API Best Practices](docs/patterns/api-best-practices.md) for route handlers
2. Use [Dependency Injection Pattern](docs/patterns/dependency-injection.md) for service setup
3. Follow [Schema Design Patterns](docs/patterns/schema-design-patterns.md) for validation

### 3. Building React components or frontend features
1. Use [React Query Patterns](docs/patterns/react-query.md) for data fetching
2. Apply [Derived State Pattern](docs/patterns/derived-state.md) for computed values
3. Follow [Error Handling Pattern](docs/patterns/error-handling.md) for error states
```

- Reference patterns from CLAUDE.md task triggers
- Maintain navigation consistency
- Update CLAUDE.md when adding new patterns
- Ensure README.md task sections align with CLAUDE.md triggers

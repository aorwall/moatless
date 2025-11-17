# Writing Skill Resources for AI Agents

## Purpose

Skill resource documents serve as operating instructions for AI agents working in this codebase. Unlike human documentation, these resources must be precise, unambiguous, and immediately actionable for AI consumption.

This guide covers writing both:
- **SKILL.md files** - Main skill entry points (<500 lines)
- **Resource files** - Detailed content in resources/ directory

---

## Table of Contents

- [SKILL.md File Structure](#skillmd-file-structure)
- [Resource File Structure](#resource-file-structure)
- [Content Principles for AI Consumption](#content-principles-for-ai-consumption)
- [Code Examples Strategy](#code-examples-strategy)
- [Correct/Wrong Pattern Examples](#correctwrong-pattern-examples)
- [Avoiding AI Confusion](#avoiding-ai-confusion)
- [Cross-Reference Strategy](#cross-reference-strategy)
- [Executable Elements](#executable-elements)
- [Validation Patterns](#validation-patterns)
- [Maintenance Guidelines](#maintenance-guidelines)

---

## SKILL.md File Structure

### Required Frontmatter

```markdown
---
name: skill-name
description: Brief description including keywords that trigger this skill. Mention topics, file types, and use cases. Be explicit about trigger terms. Include ALL keywords that should activate this skill.
---
```

- **Name**: Lowercase, hyphens, gerund form preferred (e.g., "processing-pdfs", "backend-dev-guidelines")
- **Description**: Max 1024 chars, include ALL trigger keywords
- **Must be valid YAML** - Test with a YAML parser

### Main Structure Template

```markdown
---
name: my-skill
description: Description with keywords...
---

# My Skill Title

## Purpose
What this skill helps with (1-2 sentences)

## When to Use This Skill
Bullet list of scenarios when skill activates

## Quick Reference

| Need to... | See |
|------------|-----|
| Task 1 | [Resource Name](resources/resource-file.md) |
| Task 2 | [Resource Name](resources/resource-file.md) |

## Core Principles

### 1. Principle Name
Brief explanation

### 2. Another Principle
Brief explanation

## Critical Anti-Patterns to Avoid

### Category
❌ **NEVER** do this
```language
// WRONG
```

✅ **CORRECT** approach
```language
// CORRECT
```

## Quick Start Checklist

### Before Starting Task
- [ ] Checklist item
- [ ] Checklist item

## Navigation Guide

### Essential Resources (Read First)
1. **[Resource Name](resources/file.md)** - Description

### Implementation Guides
2. **[Resource Name](resources/file.md)** - Description

## Getting Help

### When Working On...
- **Topic**: See [Resource Name](resources/file.md)
```

### Section Requirements

- **MUST** use ## for major concepts
- **MUST** use ### for specific implementations
- **MUST NOT** exceed 4 levels of nesting
- **MUST** keep total under 500 lines
- **MUST** include Quick Reference table linking to resources
- **MUST NOT** link to docs/patterns/ (skills are self-contained)
- Each section **MUST** contain actionable content

---

## Resource File Structure

### Title Convention

```markdown
# Resource Name
```

- MUST use singular nouns (e.g., "Component Pattern", not "Component Patterns")
- MUST be descriptive and specific
- MUST avoid generic terms like "Guidelines" or "Best Practices"

### Section Hierarchy

```markdown
# Resource Name

## Primary Concept
### Specific Implementation

## Another Primary Concept
### Another Implementation
```

- MUST use ## for major concepts
- MUST use ### for specific implementations
- MUST NOT exceed 4 levels of nesting
- Each section MUST contain actionable content

### Table of Contents (for files >100 lines)

```markdown
# Resource Name

## Table of Contents

- [Primary Concept](#primary-concept)
- [Another Concept](#another-concept)
  - [Subsection](#subsection)
- [Related Resources](#related-resources)

---

## Primary Concept
...
```

- MUST include TOC for files >100 lines
- MUST use anchor links matching headings
- SHOULD use horizontal rule (---) after TOC

---

## Content Principles for AI Consumption

### Imperative Directives

Use explicit directive language that AI agents can parse as rules:

```markdown
- MUST use @injectable() decorator
- MUST NOT include circular dependencies
- SHOULD provide default values for optional parameters
- MAY use either approach depending on context
- NEVER create files in the root directory
- ALWAYS validate input with Zod schemas
```

**Directive Hierarchy:**
- `MUST` / `MUST NOT` - Hard requirements, violations indicate errors
- `SHOULD` / `SHOULD NOT` - Strong recommendations, exceptions require justification
- `MAY` - Optional approaches, context-dependent
- `NEVER` / `ALWAYS` - Absolute rules with no exceptions

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
- MUST specify scope ("In controller classes...", "For API endpoints...")
- MUST provide explicit context (AI agents cannot infer unstated assumptions)

---

## Code Examples Strategy

Resource documents should include code examples **strategically** to maximize clarity while minimizing verbosity.

### Choosing Example Format

Use this decision flow to determine the appropriate example format:

```
START: I need to document a rule...

├─ Is it organizational or self-explanatory?
│  └─ YES → Omit code examples (use text description only)
│
├─ Is the syntax simple and unambiguous?
│  └─ YES → Use inline code snippets (e.g., `const foo = bar`)
│
└─ Does it involve complex interactions or multiple steps?
   └─ YES → Provide full code example blocks
```

### When Code Examples Are REQUIRED

MUST provide full code examples when:
- Rule involves syntax that could be misinterpreted by AI agents
- Multiple valid approaches exist and one is preferred
- Pattern contradicts common practices or intuition
- Implementation involves complex interaction between components
- The wrong approach is syntactically valid but semantically incorrect

### When Inline Snippets Are SUFFICIENT

MAY use inline code fragments when:
- Rule references a specific API, class, or function name
- Simple syntax changes (e.g., `MUST use const not var`)
- File path or import statement examples
- Method signatures or function calls

### When Examples Can Be OMITTED

MAY omit code examples when:
- Rule is purely organizational (e.g., "files MUST go in `/routes/`")
- Directive is self-explanatory (e.g., "MUST use TypeScript")
- Reference to existing codebase example is sufficient
- Summary table provides the information clearly

### Code Example Formats

**Format 1: Grouped Examples** (preferred for related rules)

```markdown
### Route Handler Rules

- MUST use async/await
- MUST extract params with `c.req.valid()`
- MUST handle errors with try/catch

```typescript
// ✅ CORRECT - All rules applied
app.openapi(route, async (c) => {
  try {
    const { id } = c.req.valid('param')
    return c.json(await services.get(id))
  } catch (error) {
    handleServiceError(error)
  }
})
```
```

**Format 2: Inline Snippets** (for simple rules)

```markdown
### Naming Conventions

- MUST use kebab-case: `my-component.tsx`, `task-service.ts`
- MUST use named exports: `export class Foo {}` (NEVER `export default`)
```

**Format 3: Reference Links** (for existing examples)

```markdown
### Dependency Injection

- MUST inject services via constructor
- See implementation at `apps/backend/src/services/task.service.ts:15-20`
```

**Format 4: Summary Tables** (for reference data)

```markdown
### HTTP Status Codes

| Operation | Status | Method |
|-----------|--------|--------|
| POST (create) | 201 | `c.json(data, 201)` |
| GET/PUT/PATCH | 200 | `c.json(data)` |
| DELETE | 204 | `c.body(null, 204)` |
```

### Code Example Quality Requirements

When providing code examples:
- MUST be syntactically valid and type-check correctly
- SHOULD use realistic variable names from the domain
- SHOULD include necessary imports when context is ambiguous
- MAY omit obvious boilerplate (imports, type definitions) when indicated with a comment (e.g., `// ... imports omitted`)
- MUST use actual class/function names from the codebase when possible

### Documentation Guidance

- SHOULD reference JSDoc only when it adds information beyond what TypeScript types already convey (e.g. documenting units, side effects, or invariants).
- MUST show the JSDoc block before decorators to match ts.dev style.
- MUST NOT encourage `@ts-ignore` or other comment-based suppression unless paired with an explicit rationale.

---

## Correct/Wrong Pattern Examples

Use contrasting examples **only when they provide high value** for AI disambiguation.

### When Contrasts Are REQUIRED

MUST provide CORRECT/WRONG contrasting examples when:
- Common anti-patterns exist that AI agents might follow
- The wrong approach is syntactically valid but semantically problematic
- Previous patterns or external guides recommend the wrong approach
- The difference between correct and wrong is subtle but critical

**Example - High value contrast:**

```typescript
// ✅ CORRECT - Dependencies flow down layers
import { TaskService } from '../services/task.service'

// ❌ WRONG - Dependencies flow up layers
import { registerTaskRoutes } from '../routes/tasks' // Service importing Route
```

### When Contrasts Can Be OMITTED

MAY omit WRONG examples when:
- The correct pattern is self-explanatory
- TypeScript compiler or linter would catch the error
- The rule is organizational rather than syntactic
- A description of what to avoid is sufficient

### Alternative to WRONG Examples

For simple prohibitions, use inline format instead of code blocks:

```markdown
- MUST use relative imports (`../services/foo`) NEVER absolute paths (`apps/backend/src/services/foo`)
- MUST use named exports (`export class Foo {}`) NEVER default exports (`export default`)
```

### Contrast Quality Requirements

When providing contrasting examples:
- MUST label examples with `✅ CORRECT` and `❌ WRONG` (emoji + text for visual clarity and searchability)
- MUST explain WHY the wrong example is incorrect
- SHOULD include multiple WRONG variations when common mistakes have different forms
- MAY use inline comments to highlight specific problems in WRONG examples

---

## Avoiding AI Confusion

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

---

## Cross-Reference Strategy

### Within Skill Resources

```markdown
- See [Error Handling Pattern](error-handling.md) for error response patterns
- Cross-reference with [Dependency Injection](dependency-injection.md) for service setup
- Review [Cache Invalidation](cache-invalidation.md) for data synchronization
```

- MUST reference other resources within the same skill
- MUST be specific about which section to reference
- MUST NOT create circular dependencies
- MUST NOT link to docs/patterns/ (skills are self-contained)

### Across Skills

```markdown
- For frontend data fetching patterns, see the **frontend-dev-guidelines** skill
- For backend API patterns, see the **backend-dev-guidelines** skill
- For testing approaches, see the **testing-guidelines** skill
```

- MAY reference other skills when necessary
- SHOULD be brief and let skill activation handle details
- MUST mention skill name explicitly

---

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
- SHOULD show expected output when:
  - Command produces important feedback
  - Output confirms success/failure
  - Exit codes or error messages are significant

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

---

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

---

## Maintenance Guidelines

### Pattern Evolution

```markdown
## Updating This Resource

When modifying this resource:
1. Update all code examples to match changes
2. Verify cross-referenced resources remain consistent
3. Test example code compiles and runs
4. Update Quick Reference table in SKILL.md if needed
5. Keep file under reasonable size (split if >500 lines)
```

### Version Control

- Update resource files when patterns evolve
- Test all code examples after updates
- Verify cross-references still work
- Update SKILL.md Quick Reference if resource structure changes

---

## Integration with SKILL.md

Resource files MUST integrate with the main SKILL.md navigation:

### Quick Reference Table

In SKILL.md:
```markdown
## Quick Reference

| Need to... | See |
|------------|-----|
| Create API endpoints | [API Contract Pattern](resources/api-contract-pattern.md) |
| Set up dependency injection | [Dependency Injection](resources/dependency-injection.md) |
| Validate data | [Schema Validation](resources/schema-validation.md) |
```

### Navigation Guide

In SKILL.md:
```markdown
## Navigation Guide

### Essential Resources (Read First)
1. **[API Contract Pattern](resources/api-contract-pattern.md)** - OpenAPI-first design
2. **[Layered Architecture](resources/layered-architecture.md)** - Layer responsibilities
3. **[Dependency Injection](resources/dependency-injection.md)** - InversifyJS setup

### Implementation Guides
4. **[Route Handlers](resources/route-handlers.md)** - Creating HTTP endpoints
5. **[Service Layer](resources/service-layer.md)** - Business logic patterns
6. **[Repository Pattern](resources/repository-pattern.md)** - Database access
```

- Reference resources from SKILL.md navigation sections
- Maintain clear organization in SKILL.md
- Update SKILL.md when adding/removing resources
- Ensure Quick Reference table is complete

---

## Project-Specific Guidelines

### Technology Stack

When writing for this project, reference:
- **Frontend**: React 19, TanStack Query/Router, Tailwind v4, shadcn/ui
- **Backend**: Hono, InversifyJS, Drizzle ORM, Bun runtime
- **Testing**: Vitest, React Testing Library
- **Build Tools**: Vite (frontend), TypeScript Compiler (backend)

### Code Style

- Use TypeScript strict mode
- NEVER use `any` type
- NEVER use `import type` (regular imports only)
- NEVER use default exports (named exports only)
- Use kebab-case for file names
- Follow Biome formatting rules

### File Organization

```
.claude/skills/{skill-name}/
├── SKILL.md                    # <500 lines
└── resources/
    ├── core-concept.md         # Detailed content
    ├── implementation.md       # Implementation details
    └── complete-examples.md    # Full examples
```

---

## Examples from This Project

### Frontend Skill
- **SKILL.md**: 13 resource links, Quick Reference table, <500 lines
- **Resources**: component-patterns.md, data-fetching.md, routing.md, styling.md, etc.
- **Self-contained**: No links to docs/patterns/

### Backend Skill
- **SKILL.md**: 13 resource links, Quick Reference table, <500 lines
- **Resources**: api-contract-pattern.md, dependency-injection.md, layered-architecture.md, etc.
- **Self-contained**: No links to docs/patterns/

### Testing Skill
- **SKILL.md**: 4 resource links, Quick Reference table, <500 lines
- **Resources**: testing-philosophy.md, tier-classification.md, strategic-mocking.md, etc.
- **Self-contained**: No links to docs/patterns/

---

## Summary Checklist

### For SKILL.md Files
- [ ] Valid YAML frontmatter with name and description
- [ ] Description includes all trigger keywords
- [ ] File is under 500 lines
- [ ] Quick Reference table links to all resources
- [ ] Navigation Guide organizes resources logically
- [ ] Core Principles section explains key concepts
- [ ] Critical Anti-Patterns section with examples
- [ ] No external links to docs/patterns/

### For Resource Files
- [ ] Clear title (singular noun, descriptive)
- [ ] Table of contents for files >100 lines
- [ ] Imperative directives (MUST, SHOULD, MAY)
- [ ] Strategic code examples (not excessive)
- [ ] Correct/Wrong examples where valuable
- [ ] Cross-references to other skill resources
- [ ] Executable commands with descriptions
- [ ] File paths and project structure accurate
- [ ] Implementation checklists where appropriate
- [ ] Self-contained (no docs/patterns/ links)

---

**This guide ensures skill resources are optimized for AI agent consumption while maintaining clarity and actionability.**

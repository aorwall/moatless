# TypeScript Modules

## Export Rules

### Named Exports Only

- MUST export every route, service, hook, and helper via named exports.
- MUST NOT use `export default`. It hides the canonical name and breaks refactors.
- MUST keep export lists minimal—only surface the API that other layers use.

```typescript
// CORRECT – explicit named exports
export const registerTaskRoutes = (app: OpenAPIHono<AppBindings>) => {
  // ...
}

export class TaskService {}

// WRONG – default export obscures the canonical name
export default class TaskService {}
```

### Avoid Container Classes

- MUST export functions, constants, and classes individually rather than wrapping them in a static container class.

```typescript
// CORRECT
export const TASK_CACHE_TTL_MS = 60_000
export function buildTaskCacheKey(taskId: string) {
  return `task:${taskId}`
}

// WRONG
export class TaskCacheUtils {
  static TTL_MS = 60_000
  static buildKey(taskId: string) {
    return `task:${taskId}`
  }
}
```

### Immutable API Surface

- MUST avoid `export let` and other mutable exports. If callers need a value that changes, provide a getter function.

```typescript
let workspaceMode: WorkspaceMode = 'local'

export function setWorkspaceMode(next: WorkspaceMode) {
  workspaceMode = next
}

export function getWorkspaceMode(): WorkspaceMode {
  return workspaceMode
}

// WRONG – reassignable export leaks mutation timing between modules
export let workspaceMode: WorkspaceMode = 'local'
```

## Import Paths

- MUST use relative imports (`./foo`, `../services/foo.service`) for files in the same logical package.
- MAY use configured aliases (e.g. `@moatless/types`) for shared packages published via tsconfig paths.
- SHOULD limit `../..` traversals—consider reorganizing folders when paths become unwieldy.

```typescript
// CORRECT – explicit project-relative imports
import { handleServiceError } from '../utils/error-handler'
import { registerWorkspaceRoutes } from './workspace.routes'

// WRONG – absolute path circumvents layering intent
import { handleServiceError } from 'apps/backend/src/utils/error-handler'
```

## Import Forms

### Module vs Destructuring Imports

- SHOULD use `import * as foo` when you need a namespaced surface (large API, avoids collisions).
- SHOULD use destructuring imports when a handful of symbols are referenced frequently.
- MUST keep namespace imports lowerCamelCase (`import * as taskService`), even if the file name is `task-service`.

```typescript
// CORRECT – namespace import exposes cohesive API
import * as taskRepo from '../repositories/task.repository'

const task = await taskRepo.findById(id)

// CORRECT – destructured for frequently-used helpers
import { describe, it, expect } from 'vitest'

// WRONG – namespace name is capitalized and conflicts with type name
import * as TaskService from '../services/task.service'
```

### Renaming Imports

- MAY alias imports (`import { from as observableFrom }`) when avoiding collisions or clarifying intent.
- SHOULD rename at the export site instead of importing with obtuse aliases when possible.

## Type-Only Imports

- MUST avoid `import type` / `export type` for local modules. Regular imports cover both value and type usage without churn.
- MAY split APIs into value-only or type-only modules when you truly need to forbid runtime coupling.

```typescript
// CORRECT – standard import for types and values
enum WorkspaceStatus { Ready = 'ready' }

import { WorkspaceStatusResponseSchema, WorkspaceStatus } from '@moatless/types'

// WRONG – redundant type modifiers add friction
import type { WorkspaceStatus } from '@moatless/types'
export type { WorkspaceStatus } from '@moatless/types'
```

## Decorators and Documentation Ordering

- MUST place documentation comments above decorators.
- MUST keep decorators adjacent to the symbol (no blank line).

```typescript
/** Public routes for viewing workspaces. */
@Controller('workspace')
export class WorkspaceController {}

// WRONG – decorator separated from class
@Controller('workspace')

/** misplaced */
export class WorkspaceController {}
```

## Disallowed Patterns

- MUST NOT declare `namespace Foo {}` blocks. Use modules with imports/exports instead.
- MUST NOT use CommonJS `require` assignments (`import x = require('...')`). Stick to ECMAScript module syntax.
- MUST NOT rely on `/// <reference>` directives; add explicit imports instead.

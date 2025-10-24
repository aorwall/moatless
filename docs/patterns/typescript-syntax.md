# TypeScript Syntax

## Identifier Rules

### Naming Matrix

| Style | Targets |
| --- | --- |
| `UpperCamelCase` | components, classes, interfaces, enums, decorators, type parameters |
| `lowerCamelCase` | variables, functions, hooks, namespace imports |
| `CONSTANT_CASE` | exported constants, enum members |

- MUST treat abbreviations as whole words (`loadHttpUrl`, not `loadHTTPURL`).
- MUST reserve `$` prefixes for libraries that require it (e.g. RxJS observables). Everything else stays alphanumeric.
- MUST NOT prefix or suffix identifiers with `_`, including placeholders for unused parameters. Use destructuring holes instead (`const [first,,third] = tuple`).
- SHOULD keep namespace imports lowerCamelCase even when the file name is snake_case.

```typescript
// CORRECT
export interface TaskDetailProps {
  readonly taskId: string
  readonly workspaceId: string
}

const useTaskDetail = (taskId: string) => {
  // ...
}

// WRONG – casing and sentinels violate the style guide
export interface task_detail_props {
  readonly Task_Id: string
}

export function render_task_detail(_task: Task) {
  // ...
}
```

## File Encoding

- MUST save all source files as UTF-8 without BOM.
- MUST keep files ASCII-only unless domain requirements mandate Unicode (e.g. localized copy).

## Formatting Essentials

### Blocks and Control Flow

- MUST wrap multi-line control-flow bodies in braces, even if JavaScript would allow single-line statements.
- MAY omit braces only when the entire `if` statement fits on one line.
- MUST indent decorators directly above the symbol they decorate, with any JSDoc comment above the decorator.

```typescript
// CORRECT – braces and decorator placement
if (task.isBlocked) {
  notifySlack(task.id)
}

/** Marks workspace routes. */
@Controller('workspace')
export class WorkspaceController {}

// WRONG – missing braces and misplaced JSDoc
if (task.isBlocked)
  notifySlack(task.id)

@Controller('workspace')
/** misplaced */
export class WorkspaceController {}
```

### Equality and Booleans

- MUST use `===` and `!==` for comparisons.
- MUST rely on implicit boolean coercion in conditionals or compare explicitly; NEVER double-negate inside `if`, `for`, or `while` conditions.

```typescript
if (items.length === 0) {
  return 'empty'
}

// WRONG
if (!!items.length) {
  // double negation obscures intent
}
```

## Type-Safe Coercions

- MUST convert to numbers with `Number(value)` (optionally followed by `Math.trunc`/`Math.floor`).
- MUST guard against `Number` returning `NaN` when parsing user input.
- MUST convert to boolean with `Boolean(value)` or double-negation outside conditionals (e.g. storing flags). Inside conditionals, rely on implicit coercion.
- MUST convert to string with template literals or `String(value)`.
- MUST NOT use unary `+value`, `parseInt`, or `parseFloat` for general parsing.

```typescript
// CORRECT
const limitInput = Number(form.limit)
if (Number.isNaN(limitInput)) {
  throw new Error('Limit must be numeric')
}

const payload = String(request.body.id)
const isReady = Boolean(job)

// WRONG – silent truncation and ambiguous coercion
const limit = +form.limit
const value = parseFloat(form.limit)
```

## Spread Safety

- MUST spread only compatible types (objects into objects, arrays into arrays).
- MUST guard optional spreads with default objects to avoid spreading `undefined` or `null`.

```typescript
const taskView = {
  ...baseTask,
  status,
}

const options = shouldApplyTheme ? { ...themeOptions } : {}

// WRONG – spreading null/primitive
const taskView2 = { ...baseTask.settings } // settings may be null
const ids = { ...taskIds } // spreads array into object
```

## Comment and JSDoc Discipline

- SHOULD write JSDoc only when it adds information beyond the type signature (units, side effects, invariants).
- MUST place JSDoc above decorators or symbols.
- MUST NOT use `@ts-ignore` to hide type errors; prefer proper typing or `@ts-expect-error` with justification when failure is deliberate.

```typescript
/**
 * Starts workspace provisioning.
 * @param workspaceId UUID from the backend contract.
 */
export async function startWorkspace(workspaceId: string) {
  // ...
}

// WRONG – redundant comment and ts-ignore
/** @param workspaceId string */
// @ts-ignore
export async function startWorkspace(workspaceId) {
  // ...
}
```
```

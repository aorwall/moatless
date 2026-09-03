---
name: frontend-dev-guidelines
description: Moatless Vibe frontend conventions — React 19 + TanStack Query/Router, Tailwind v4 + shadcn/ui, the generated @moatless/api-client, feature-based structure. Use when building or changing frontend components, routes, data fetching, styling, or frontend tests.
---

# Frontend Development Guidelines

Repo-specific patterns for `apps/frontend`. Baseline conventions for type safety, server state, file organization, and task-page layers live in this skill and its resources.

## Resources

| Topic | File |
|---|---|
| Data fetching (TanStack Query + generated client) | [data-fetching.md](resources/data-fetching.md) |
| Routing (TanStack Router, file-based) | [routing.md](resources/routing.md) |
| Styling (Tailwind v4, semantic tokens, `cn()`) | [styling.md](resources/styling.md) |
| Error handling | [error-handling.md](resources/error-handling.md) |
| Testing | [testing.md](resources/testing.md) |
| Storybook | [storybook.md](resources/storybook.md) |
| Configuration/settings dialogs | [configuration-dialog.md](resources/configuration-dialog.md) |
| TypeScript patterns | [typescript-patterns.md](resources/typescript-patterns.md) |
| Complete examples | [complete-examples.md](resources/complete-examples.md) |

## Structure

```
apps/frontend/src/
├── features/            # admin, agents, auth, events, feedback, files, integrations,
│   │                    # navigation, plugins, preview, repositories, run-config, sandbox,
│   │                    # secrets, settings, setup, skills, tags, task-connections,
│   │                    # tasks, uploads, workspaces
│   └── {name}/          # api/ components/ hooks/  (+ contexts/ layouts/ panels/ in tasks/)
├── lib/
│   ├── api/             # client.ts, query-client.ts, query-keys.ts
│   ├── components/ui/   # shadcn/ui components
│   ├── styles/globals.css   # Tailwind v4 config (@theme, semantic tokens)
│   └── utils/           # cn(), helpers
├── routes/              # __root.tsx, index.tsx, dashboard.tsx, login.tsx, settings.tsx,
│                        # secrets.lazy.tsx, tasks/, admin/, auth/, oauth/, sandbox/
└── test-utils/          # render.tsx (custom render), setup.ts
```

## API integration

1. New/changed backend endpoint → `bun run generate:api` (dumps the OpenAPI spec, runs Orval → `@moatless/api-client`).
2. Wrap the generated fetcher in a feature `api/` hook: `queryFn: () => unwrapResponse(listTasks())`.
3. Query keys come from `@/lib/api/query-keys.ts` — never inline arrays.
4. Never hand-roll `fetch('/api/v1/...')`.

## Key facts

- `import type` for type-only imports — Biome `useImportType` is an error.
- Toasts: `import { toast } from 'sonner'`.
- Colors: semantic tokens (`bg-background`, `text-muted-foreground`) — never raw scales (`bg-blue-600`). Merge classes with `cn()` from `@/lib/utils`.
- Tests: render via `@/test-utils/render`; dominant naming is `*.unit.test.tsx`.
- New shadcn component: `cd apps/frontend && bunx shadcn@latest add [component]`.

## Comments

The comment rules in the root `AGENTS.md` apply to every TypeScript file in the
repo, including `packages/` and `tests/`. TypeScript specifics:

- Doc comments are TSDoc Markdown: `-` lists, fenced blocks, and a bold label
  such as `**Failure policy:**` are the house style. No HTML tags.
- `@param`, `@returns`, and `@throws` carry only what the type cannot express:
  a unit, a range, an ordering, who owns a callback. A `@param` that restates
  the type is a cut.
- A React prop is documented on the prop in its props type, never with `@param`
  on the component.
- Link to the owner of a fact with `{@link Type.member}`. Biome does not check
  links; review does.
- A `biome-ignore` reason states the cause only.
- A Zod schema's doc states a constraint the schema does not encode. "X schema"
  above `XSchema` is a cut.
- A test comment that restates the test title or the assertion below it is a
  cut. A `// ─── ID: title ───` banner in `tests/api` indexes a scenario the
  file-top doc lists and stays.

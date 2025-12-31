# Plan: Rename Repository Pattern to DAO Pattern

## Summary
Rename "Repository" to "DAO" (Data Access Object) throughout the codebase:
- File names: `*.repository.ts` → `*.dao.ts`
- Class names: `*Repository` → `*Dao`
- Layer name: "Repository Layer" → "Data Access Layer"

---

## Scope

### 11 Repository Files to Rename

| Current File | New File | Class Rename |
|--------------|----------|--------------|
| `features/tasks/db/task.repository.ts` | `task.dao.ts` | `TaskRepository` → `TaskDao` |
| `features/tasks/db/task-message.repository.ts` | `task-message.dao.ts` | `TaskMessageRepository` → `TaskMessageDao` |
| `features/auth/db/user.repository.ts` | `user.dao.ts` | `UserRepository` → `UserDao` |
| `features/auth/db/session.repository.ts` | `session.dao.ts` | `SessionRepository` → `SessionDao` |
| `features/auth/db/provider-token.repository.ts` | `provider-token.dao.ts` | `ProviderTokenRepository` → `ProviderTokenDao` |
| `features/auth/db/oauth-state.repository.ts` | `oauth-state.dao.ts` | `OAuthStateRepository` → `OAuthStateDao` |
| `features/auth/db/api-key.repository.ts` | `api-key.dao.ts` | `ApiKeyRepository` → `ApiKeyDao` |
| `features/repositories/db/repository.repository.ts` | `repository.dao.ts` | `RepositoryRepository` → `RepositoryDao` |
| `features/workspace/db/workspace-runtime.repository.ts` | `workspace-runtime.dao.ts` | `WorkspaceRuntimeRepository` → `WorkspaceRuntimeDao` |
| `features/agents/db/command.repository.ts` | `command.dao.ts` | `CommandRepository` → `CommandDao` |
| `features/events/db/event.repository.ts` | `event.dao.ts` | `EventRepository` → `EventDao` |

---

## Implementation Steps

### Step 1: Rename Files and Classes
For each of the 11 repository files:
1. Rename file: `*.repository.ts` → `*.dao.ts`
2. Rename class: `*Repository` → `*Dao`
3. Update exports

### Step 2: Update DI Container
File: `apps/backend/src/container/container.ts`
- Update all class imports and bindings
- Update string symbol: `'ProviderTokenRepository'` → `'ProviderTokenDao'`

### Step 3: Update Service Imports
Update all services that inject repositories:
- Change import paths
- Change type annotations
- Change `@inject()` decorators

### Step 4: Update Documentation

**Architecture docs:**
- `docs/architecture-overview.md`: "REPOSITORY LAYER" → "DATA ACCESS LAYER"

**CLAUDE.md:**
- "repositories" → "DAOs"
- "Repository Pattern" → "DAO Pattern"

**Skills documentation:**
- Rename `.claude/skills/backend-dev-guidelines/resources/repository-pattern.md` → `dao-pattern.md`
- Update `.claude/skills/backend-dev-guidelines/SKILL.md`
- Update `.claude/skills/backend-dev-guidelines/resources/service-layer.md`

### Step 5: Update Schema Comments
Files with "repository layer" comments:
- `features/auth/db/oauth-state.schema.ts`
- `features/auth/db/pg/oauth-state.schema.ts`
- `features/auth/db/provider-token.schema.ts`
- `features/auth/db/pg/provider-token.schema.ts`

### Step 6: Verify
```bash
bun run type:check
bun run biome:check
bun run test
```

---

## Files to Modify (Complete List)

### DAO Files (rename + edit)
```
apps/backend/src/features/tasks/db/task.repository.ts
apps/backend/src/features/tasks/db/task-message.repository.ts
apps/backend/src/features/auth/db/user.repository.ts
apps/backend/src/features/auth/db/session.repository.ts
apps/backend/src/features/auth/db/provider-token.repository.ts
apps/backend/src/features/auth/db/oauth-state.repository.ts
apps/backend/src/features/auth/db/api-key.repository.ts
apps/backend/src/features/repositories/db/repository.repository.ts
apps/backend/src/features/workspace/db/workspace-runtime.repository.ts
apps/backend/src/features/agents/db/command.repository.ts
apps/backend/src/features/events/db/event.repository.ts
```

### Container
```
apps/backend/src/container/container.ts
```

### Services (imports/types)
All services in `features/*/services/` that inject repositories

### Documentation
```
docs/architecture-overview.md
CLAUDE.md
.claude/skills/backend-dev-guidelines/SKILL.md
.claude/skills/backend-dev-guidelines/resources/repository-pattern.md → dao-pattern.md
.claude/skills/backend-dev-guidelines/resources/service-layer.md
```

### Schema Comments
```
apps/backend/src/features/auth/db/oauth-state.schema.ts
apps/backend/src/features/auth/db/pg/oauth-state.schema.ts
apps/backend/src/features/auth/db/provider-token.schema.ts
apps/backend/src/features/auth/db/pg/provider-token.schema.ts
```

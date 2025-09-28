# Settings Management Pattern

This document describes the settings management pattern used in the Moatless Vibe UI application, demonstrating how settings are organized into logical sections with dedicated API endpoints.

## Overview

The settings system is designed with the following principles:
- **Separation of Concerns**: Each settings section (general, GitHub, Claude) has its own endpoint
- **Type Safety**: Zod schemas ensure runtime validation and TypeScript types
- **Optimistic Updates**: React Query mutations provide immediate UI feedback
- **Partial Updates**: Only send changed fields to reduce payload size
- **Backward Compatibility**: Legacy endpoints maintained during migration

## Architecture

### Backend Structure

#### 1. Schema Organization
Settings schemas are organized into separate files for each section:

```typescript
// packages/types/src/schemas/settings/general.schemas.ts
export const GeneralSettingsSchema = z.object({
  dataDir: z.string().optional(),
  defaultWorkspaceType: z.enum(['docker', 'local']).optional(),
})

// packages/types/src/schemas/settings/github.schemas.ts
export const GitHubSettingsSchema = z.object({
  personalAccessToken: z.string().nullable().optional(),
  userName: z.string().nullable().optional(),
  userEmail: z.string().nullable().optional(),
})

// packages/types/src/schemas/settings/claude.schemas.ts
export const ClaudeSettingsSchema = z.object({
  binDir: z.string().optional(),
  configDir: z.string().optional(),
  oauthToken: z.string().optional(),
})
```

#### 2. Route Organization
Each settings section has its own route file:

```typescript
// apps/backend/routes/settings/general.ts
export function registerGeneralSettingsRoutes(app: Hono<AppBindings>) {
  app.get('/', async (c) => {
    const settings = c.get('serviceContainer').settings
    return c.json(settings.getGeneralSettings())
  })

  app.put('/', zValidator('json', UpdateGeneralSettingsSchema), async (c) => {
    const settings = c.get('serviceContainer').settings
    const request = c.req.valid('json')
    const response = await settings.updateGeneralSettings(request)
    return c.json(response)
  })
}
```

#### 3. Service Methods
The settings service provides section-specific methods:

```typescript
class SettingsService {
  // Section-specific getters
  getGeneralSettings(): GeneralSettingsResponse {
    return {
      settings: {
        dataDir: this.settings.dataDir,
        defaultWorkspaceType: this.settings.defaultWorkspaceType,
      },
      defaults: {
        dataDir: this.defaultSettings.dataDir,
        defaultWorkspaceType: this.defaultSettings.defaultWorkspaceType,
      },
    }
  }

  // Section-specific updates
  async updateGeneralSettings(request: UpdateGeneralSettingsRequest): Promise<GeneralSettingsResponse> {
    // Only update provided fields
    if (request.dataDir !== undefined) {
      this.settings.dataDir = request.dataDir || this.defaultSettings.dataDir
    }
    if (request.defaultWorkspaceType !== undefined) {
      this.settings.defaultWorkspaceType = request.defaultWorkspaceType || this.defaultSettings.defaultWorkspaceType
    }
    
    await this.saveSettings()
    return this.getGeneralSettings()
  }
}
```

### Frontend Structure

#### 1. API Layer
Organized API methods for each section:

```typescript
// apps/frontend/src/features/settings/api/settings.ts
export const settingsApi = {
  general: {
    get: async (): Promise<GeneralSettingsResponse> => {
      return apiClient.get<GeneralSettingsResponse>('/api/v1/settings/general')
    },
    update: async (request: UpdateGeneralSettingsRequest): Promise<GeneralSettingsResponse> => {
      return apiClient.put<GeneralSettingsResponse>('/api/v1/settings/general', request)
    },
  },
  github: {
    get: async (): Promise<GitHubSettingsResponse> => {
      return apiClient.get<GitHubSettingsResponse>('/api/v1/settings/github')
    },
    update: async (request: UpdateGitHubSettingsRequest): Promise<GitHubSettingsResponse> => {
      return apiClient.put<GitHubSettingsResponse>('/api/v1/settings/github', request)
    },
  },
  // ... other sections
}
```

#### 2. React Query Hooks
Dedicated hooks for each settings section:

```typescript
// apps/frontend/src/features/settings/hooks/use-github-settings.ts
export function useGitHubSettings() {
  return useQuery<GitHubSettingsResponse>({
    queryKey: queryKeys.settings.github(),
    queryFn: settingsApi.github.get,
  })
}

export function useUpdateGitHubSettings(
  options?: UseMutationOptions<GitHubSettingsResponse, ApiError, UpdateGitHubSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<GitHubSettingsResponse, ApiError, UpdateGitHubSettingsRequest>({
    mutationFn: (request) => settingsApi.github.update(request),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.github(), data)
      // Invalidate legacy endpoints for backward compatibility
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.detail() })
    },
    ...options,
  })
}
```

#### 3. Component Pattern
Components follow the derived state pattern:

```typescript
export function GitHubConfigurationSection() {
  const { data: githubSettings, isLoading, error } = useGitHubSettings()
  const updateGitHubSettings = useUpdateGitHubSettings()

  // Derive base form data from settings
  const baseGitHubFormData = useMemo<UpdateGitHubSettingsRequest>(
    () => ({
      personalAccessToken: githubSettings?.settings.personalAccessToken || null,
      userName: githubSettings?.settings.userName || null,
      userEmail: githubSettings?.settings.userEmail || null,
    }),
    [githubSettings],
  )

  // Track user modifications separately
  const [githubUserModifications, setGitHubUserModifications] = useState<Partial<UpdateGitHubSettingsRequest>>({})

  // Merge for final form data
  const githubFormData = useMemo(
    () => ({
      ...baseGitHubFormData,
      ...githubUserModifications,
    }),
    [baseGitHubFormData, githubUserModifications],
  )

  // Handle save
  const handleSave = () => {
    updateGitHubSettings.mutate(githubFormData, {
      onSuccess: () => {
        setGitHubUserModifications({})
        toast.success('GitHub settings saved successfully')
      },
      onError: (error) => {
        toast.error('Failed to save GitHub settings', {
          description: getErrorMessage(error),
        })
      },
    })
  }
}
```

## API Endpoints

The settings API is organized into logical sections:

```
GET  /api/v1/settings/general     # Get general settings
PUT  /api/v1/settings/general     # Update general settings

GET  /api/v1/settings/github      # Get GitHub settings  
PUT  /api/v1/settings/github      # Update GitHub settings

GET  /api/v1/settings/claude      # Get Claude settings
PUT  /api/v1/settings/claude      # Update Claude settings

GET  /api/v1/settings             # Legacy: Get all settings
PUT  /api/v1/settings             # Legacy: Update all settings
POST /api/v1/settings/reset       # Legacy: Reset to defaults
```

## Adding a New Settings Section

To add a new settings section:

### 1. Create Schema
```typescript
// packages/types/src/schemas/settings/docker.schemas.ts
export const DockerSettingsSchema = z.object({
  socketPath: z.string().optional(),
  network: z.string().optional(),
})

export const UpdateDockerSettingsSchema = DockerSettingsSchema.partial()

export type DockerSettings = z.infer<typeof DockerSettingsSchema>
export type UpdateDockerSettingsRequest = z.infer<typeof UpdateDockerSettingsSchema>

export const DockerSettingsResponseSchema = z.object({
  settings: DockerSettingsSchema,
  defaults: DockerSettingsSchema,
})

export type DockerSettingsResponse = z.infer<typeof DockerSettingsResponseSchema>
```

### 2. Create Route Handler
```typescript
// apps/backend/routes/settings/docker.ts
import { zValidator } from '@hono/zod-validator'
import type { Hono } from 'hono'
import { UpdateDockerSettingsSchema } from '@moatless/types'
import type { AppBindings } from '@/types/app'

export function registerDockerSettingsRoutes(app: Hono<AppBindings>) {
  app.get('/', async (c) => {
    const settings = c.get('serviceContainer').settings
    return c.json(settings.getDockerSettings())
  })

  app.put('/', zValidator('json', UpdateDockerSettingsSchema), async (c) => {
    const settings = c.get('serviceContainer').settings
    const request = c.req.valid('json')
    const response = await settings.updateDockerSettings(request)
    return c.json(response)
  })
}
```

### 3. Register Route
```typescript
// apps/backend/routes/settings.ts
import { registerDockerSettingsRoutes } from './settings/docker'

const dockerRouter = new Hono<AppBindings>()
registerDockerSettingsRoutes(dockerRouter)
app.route(`${prefix}/docker`, dockerRouter)
```

### 4. Add Service Methods
```typescript
// apps/backend/services/settings.service.ts
getDockerSettings(): DockerSettingsResponse {
  return {
    settings: {
      socketPath: this.settings.docker?.socketPath,
      network: this.settings.docker?.network,
    },
    defaults: {
      socketPath: this.defaultSettings.docker?.socketPath,
      network: this.defaultSettings.docker?.network,
    },
  }
}

async updateDockerSettings(request: UpdateDockerSettingsRequest): Promise<DockerSettingsResponse> {
  if (!this.settings.docker) {
    this.settings.docker = {}
  }
  
  if (request.socketPath !== undefined) {
    this.settings.docker.socketPath = request.socketPath || this.defaultSettings.docker?.socketPath
  }
  if (request.network !== undefined) {
    this.settings.docker.network = request.network || this.defaultSettings.docker?.network
  }
  
  await this.saveSettings()
  return this.getDockerSettings()
}
```

### 5. Create Frontend API Methods
```typescript
// apps/frontend/src/features/settings/api/settings.ts
docker: {
  get: async (): Promise<DockerSettingsResponse> => {
    return apiClient.get<DockerSettingsResponse>('/api/v1/settings/docker')
  },
  update: async (request: UpdateDockerSettingsRequest): Promise<DockerSettingsResponse> => {
    return apiClient.put<DockerSettingsResponse>('/api/v1/settings/docker', request)
  },
},
```

### 6. Create React Query Hooks
```typescript
// apps/frontend/src/features/settings/hooks/use-docker-settings.ts
export function useDockerSettings() {
  return useQuery<DockerSettingsResponse>({
    queryKey: queryKeys.settings.docker(),
    queryFn: settingsApi.docker.get,
  })
}

export function useUpdateDockerSettings(
  options?: UseMutationOptions<DockerSettingsResponse, ApiError, UpdateDockerSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<DockerSettingsResponse, ApiError, UpdateDockerSettingsRequest>({
    mutationFn: (request) => settingsApi.docker.update(request),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.docker(), data)
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.detail() })
    },
    ...options,
  })
}
```

### 7. Add Query Key
```typescript
// apps/frontend/src/lib/api/query-keys.ts
settings: {
  // ... existing keys
  docker: () => [...queryKeys.settings.all(), 'docker'] as const,
},
```

## Benefits

1. **Granular Updates**: Only update the settings that changed
2. **Smaller Payloads**: Send only relevant data for each section
3. **Better Organization**: Clear separation between different configuration areas
4. **Type Safety**: Full TypeScript support with runtime validation
5. **Optimistic UI**: Immediate feedback with React Query mutations
6. **Backward Compatibility**: Legacy endpoints continue to work during migration

## Migration Strategy

1. Create new section-specific endpoints alongside legacy endpoints
2. Update frontend to use new endpoints while maintaining legacy support
3. Monitor usage of legacy endpoints
4. Deprecate and remove legacy endpoints after migration period

## Security Considerations

1. **Sensitive Data**: Use appropriate field types for secrets (e.g., password inputs)
2. **Validation**: All inputs validated with Zod schemas
3. **Sanitization**: Trim and normalize string inputs
4. **Defaults**: Sensitive fields default to null when not provided
5. **Environment Variables**: OAuth tokens and credentials passed securely to containers

## Testing

1. **Unit Tests**: Test service methods with mocked file system
2. **Integration Tests**: Test API endpoints with real service
3. **Component Tests**: Test UI components with mocked hooks
4. **E2E Tests**: Test full settings flow from UI to persistence
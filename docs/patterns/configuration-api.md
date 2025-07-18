# Configuration API Pattern

## Overview

The Configuration API pattern provides a PATCH-based approach for granular configuration updates, enabling efficient and immediate persistence of individual settings while maintaining data consistency and providing excellent user experience.

## Key Principles

- **PATCH-Based Updates**: Update only the fields that changed
- **Immediate Persistence**: Save changes as soon as they're made (with smart debouncing)
- **Schema-First Validation**: Use Zod schemas for runtime validation
- **Optimistic Updates**: Update UI immediately, rollback on error
- **Granular Endpoints**: Separate endpoints for different configuration domains

## API Design Patterns

### Endpoint Structure

```
PATCH /api/settings/general           # Application-wide settings
PATCH /api/settings/claude-code       # Claude Code specific settings
PATCH /api/repositories/{id}/config  # Repository-specific configuration
PATCH /api/environments/{id}/config  # Environment-specific configuration
```

### Request/Response Schema Pattern

```typescript
// Schema definition (in schemas/settings.schemas.ts)
export const UpdateGeneralSettingsSchema = z.object({
  dataDir: z.string().optional(),
  defaultWorkspaceType: z.enum(['local', 'docker']).optional(),
  claude: z.object({
    binDir: z.string().optional(),
    configDir: z.string().optional(),
  }).optional(),
})

export const UpdateClaudeCodeSettingsSchema = z.object({
  model: z.string().optional(),
  maxTurns: z.number().int().positive().optional(),
  maxThinkingTokens: z.number().int().min(0).optional(),
  systemPrompt: z.string().optional(),
  appendSystemPrompt: z.string().optional(),
  allowedTools: z.array(z.string()).optional(),
  disallowedTools: z.array(z.string()).optional(),
  permissionMode: z.enum(['default', 'acceptEdits', 'bypassPermissions']).optional(),
})

// Type inference
export type UpdateGeneralSettingsRequest = z.infer<typeof UpdateGeneralSettingsSchema>
export type UpdateClaudeCodeSettingsRequest = z.infer<typeof UpdateClaudeCodeSettingsSchema>
```

### Backend Implementation Pattern

```typescript
// Route handler example (apps/backend/routes/settings.ts)
import { zValidator } from '@hono/zod-validator'
import { UpdateClaudeCodeSettingsSchema } from '@moatless/types'

app.patch(
  '/settings/claude-code',
  zValidator('json', UpdateClaudeCodeSettingsSchema),
  async (c) => {
    const updates = c.req.valid('json')
    
    try {
      // Apply partial updates
      const updatedSettings = await settingsService.updateClaudeCodeSettings(updates)
      
      return c.json({
        success: true,
        data: updatedSettings,
      })
    } catch (error) {
      return c.json({
        success: false,
        error: getErrorMessage(error),
      }, 500)
    }
  }
)
```

## Frontend Integration Patterns

### React Query Mutations

```typescript
// Hook for PATCH updates (features/settings/hooks/use-update-settings.ts)
export function useUpdateClaudeCodeSettings() {
  const queryClient = useQueryClient()
  
  return useMutation({
    mutationFn: async (updates: UpdateClaudeCodeSettingsRequest) => {
      const response = await api.patch('/settings/claude-code', updates)
      return response.data
    },
    onMutate: async (updates) => {
      // Optimistic update
      await queryClient.cancelQueries({ queryKey: ['settings', 'claude-code'] })
      
      const previousSettings = queryClient.getQueryData(['settings', 'claude-code'])
      
      queryClient.setQueryData(['settings', 'claude-code'], (old: any) => ({
        ...old,
        ...updates,
      }))
      
      return { previousSettings }
    },
    onError: (error, updates, context) => {
      // Rollback on error
      if (context?.previousSettings) {
        queryClient.setQueryData(['settings', 'claude-code'], context.previousSettings)
      }
    },
    onSettled: () => {
      // Refetch to ensure consistency
      queryClient.invalidateQueries({ queryKey: ['settings', 'claude-code'] })
    },
  })
}
```

### Field-Level Updates

```typescript
// Individual field update pattern
function useFieldUpdate<T>(
  fieldPath: string,
  updateMutation: UseMutationResult<any, Error, T>
) {
  const updateField = useCallback((value: T[keyof T]) => {
    updateMutation.mutate({
      [fieldPath]: value,
    } as T)
  }, [fieldPath, updateMutation])
  
  return updateField
}

// Usage in component
function ModelSettingsField() {
  const updateSettings = useUpdateClaudeCodeSettings()
  const updateModel = useFieldUpdate('model', updateSettings)
  
  return (
    <Input
      value={currentModel}
      onChange={(e) => updateModel(e.target.value)}
      onBlur={() => {
        // Immediate save on blur for text inputs
        if (currentModel !== originalModel) {
          updateModel(currentModel)
        }
      }}
    />
  )
}
```

## Smart Debouncing for Text Areas

### Debounced Update Hook

```typescript
import { useCallback, useRef } from 'react'
import { useMutation } from '@tanstack/react-query'
import { debounce } from 'lodash-es'

export function useDebouncedUpdate<T>(
  updateMutation: UseMutationResult<any, Error, T>,
  delay = 500
) {
  const pendingUpdatesRef = useRef<Partial<T>>({})
  
  const debouncedUpdate = useCallback(
    debounce((updates: Partial<T>) => {
      const finalUpdates = { ...pendingUpdatesRef.current, ...updates }
      pendingUpdatesRef.current = {}
      updateMutation.mutate(finalUpdates)
    }, delay),
    [updateMutation, delay]
  )
  
  const updateField = useCallback((field: keyof T, value: T[keyof T]) => {
    // Store pending update
    pendingUpdatesRef.current = {
      ...pendingUpdatesRef.current,
      [field]: value,
    }
    
    // Trigger debounced save
    debouncedUpdate(pendingUpdatesRef.current)
  }, [debouncedUpdate])
  
  return updateField
}
```

### Textarea Implementation

```typescript
function SystemPromptField() {
  const updateSettings = useUpdateClaudeCodeSettings()
  const updateField = useDebouncedUpdate(updateSettings, 750) // Longer delay for text areas
  const [localValue, setLocalValue] = useState(initialValue)
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
  
  const handleChange = (value: string) => {
    setLocalValue(value)
    setSaveStatus('saving')
    updateField('systemPrompt', value)
  }
  
  useEffect(() => {
    if (updateSettings.isSuccess) {
      setSaveStatus('saved')
      setTimeout(() => setSaveStatus('idle'), 2000)
    } else if (updateSettings.isError) {
      setSaveStatus('error')
    }
  }, [updateSettings.isSuccess, updateSettings.isError])
  
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <Label>System Prompt</Label>
        <SaveStatusIndicator status={saveStatus} />
      </div>
      <Textarea
        value={localValue}
        onChange={(e) => handleChange(e.target.value)}
        rows={4}
      />
    </div>
  )
}

function SaveStatusIndicator({ status }: { status: string }) {
  switch (status) {
    case 'saving':
      return (
        <div className="flex items-center gap-1 text-xs text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin" />
          Saving...
        </div>
      )
    case 'saved':
      return (
        <div className="flex items-center gap-1 text-xs text-green-600">
          <Check className="h-3 w-3" />
          Saved
        </div>
      )
    case 'error':
      return (
        <div className="flex items-center gap-1 text-xs text-red-600">
          <AlertCircle className="h-3 w-3" />
          Error
        </div>
      )
    default:
      return null
  }
}
```

## Backend Service Implementation

### Service Layer Pattern

```typescript
// services/settings.service.ts
export class SettingsService {
  async updateClaudeCodeSettings(
    updates: UpdateClaudeCodeSettingsRequest
  ): Promise<ClaudeCodeSettings> {
    // 1. Load current settings
    const currentSettings = await this.getClaudeCodeSettings()
    
    // 2. Merge updates with current settings
    const mergedSettings = {
      ...currentSettings,
      ...updates,
    }
    
    // 3. Validate merged settings
    const validatedSettings = ClaudeCodeSettingsSchema.parse(mergedSettings)
    
    // 4. Persist to storage
    await this.persistClaudeCodeSettings(validatedSettings)
    
    // 5. Return updated settings
    return validatedSettings
  }
  
  private async persistClaudeCodeSettings(settings: ClaudeCodeSettings) {
    // File-based persistence, database, or external service
    await fs.writeFile(
      this.getSettingsPath('claude-code'),
      JSON.stringify(settings, null, 2)
    )
  }
}
```

### Error Handling Pattern

```typescript
// Structured error responses
export class ConfigurationError extends Error {
  constructor(
    message: string,
    public field?: string,
    public code?: string
  ) {
    super(message)
    this.name = 'ConfigurationError'
  }
}

// Route error handling
app.patch('/settings/claude-code', async (c) => {
  try {
    const updates = c.req.valid('json')
    const result = await settingsService.updateClaudeCodeSettings(updates)
    
    return c.json({
      success: true,
      data: result,
    })
  } catch (error) {
    if (error instanceof ConfigurationError) {
      return c.json({
        success: false,
        error: error.message,
        field: error.field,
        code: error.code,
      }, 400)
    }
    
    // Generic error
    return c.json({
      success: false,
      error: 'Internal server error',
    }, 500)
  }
})
```

## Validation Patterns

### Client-Side Validation

```typescript
// Real-time validation hook
export function useFieldValidation<T>(
  schema: z.ZodSchema<T>,
  field: keyof T
) {
  const [error, setError] = useState<string | null>(null)
  
  const validate = useCallback((value: any) => {
    try {
      schema.pick({ [field]: true }).parse({ [field]: value })
      setError(null)
      return true
    } catch (err) {
      if (err instanceof z.ZodError) {
        setError(err.errors[0]?.message || 'Invalid value')
      }
      return false
    }
  }, [schema, field])
  
  return { error, validate }
}

// Usage in component
function MaxTurnsField() {
  const { error, validate } = useFieldValidation(
    UpdateClaudeCodeSettingsSchema,
    'maxTurns'
  )
  
  return (
    <div className="space-y-2">
      <Input
        type="number"
        onChange={(e) => {
          const value = parseInt(e.target.value)
          if (validate(value)) {
            updateField('maxTurns', value)
          }
        }}
      />
      {error && (
        <p className="text-sm text-red-600">{error}</p>
      )}
    </div>
  )
}
```

### Server-Side Validation

```typescript
// Validation middleware
export function validateConfigUpdate<T>(schema: z.ZodSchema<T>) {
  return async (c: Context, next: Next) => {
    try {
      const body = await c.req.json()
      const validated = schema.parse(body)
      c.set('validatedData', validated)
      await next()
    } catch (error) {
      if (error instanceof z.ZodError) {
        return c.json({
          success: false,
          error: 'Validation failed',
          details: error.errors,
        }, 400)
      }
      throw error
    }
  }
}
```

## Performance Considerations

### 1. Debouncing Strategy
- **Text inputs**: 300ms debounce
- **Textareas**: 500-750ms debounce
- **Select/toggles**: Immediate updates
- **Numeric inputs**: Validate on blur

### 2. Optimistic Updates
- Update UI immediately for better UX
- Rollback on error with visual feedback
- Queue multiple rapid updates

### 3. Caching Strategy
- Use React Query for automatic caching
- Invalidate related queries on updates
- Consider stale-while-revalidate patterns

### 4. Network Optimization
- Batch multiple field updates when possible
- Use HTTP/2 multiplexing for concurrent requests
- Implement request deduplication

## Security Considerations

### 1. Input Validation
- Validate all inputs on both client and server
- Sanitize string inputs to prevent XSS
- Enforce business rules and constraints

### 2. Permission Checks
- Verify user permissions before updates
- Implement role-based access control
- Audit configuration changes

### 3. Rate Limiting
- Implement rate limiting for update endpoints
- Prevent abuse of debounced endpoints
- Monitor for suspicious activity patterns

## Best Practices

### 1. Schema Design
- Keep schemas focused and domain-specific
- Use optional fields for PATCH operations
- Provide clear validation error messages

### 2. Error Handling
- Provide field-specific error messages
- Implement retry logic for transient failures
- Log errors for debugging and monitoring

### 3. User Experience
- Show save status for long-running operations
- Provide immediate feedback for user actions
- Handle offline scenarios gracefully

### 4. Testing
- Test validation schemas thoroughly
- Mock API calls in frontend tests
- Test error scenarios and edge cases

## Related Patterns

- **[Schema Design Patterns](./schema-design-patterns.md)**: For Zod schema best practices
- **[API Best Practices](./api-best-practices.md)**: For general API design principles
- **[React Query Patterns](./react-query.md)**: For client-side data management
- **[Error Handling Pattern](./error-handling.md)**: For comprehensive error handling
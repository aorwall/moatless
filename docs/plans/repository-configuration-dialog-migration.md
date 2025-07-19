# Repository Configuration Dialog Migration Implementation Guide

## Task Overview

Migrate the repository configuration from individual repository edit pages to a single configuration dialog that lists all repositories as sections in the sidebar. Each repository section will contain all configuration options (general settings, code execution, and preview environment) in one scrollable view. 

The implementation will extend the existing ConfigurationDialog component to add search functionality, making it reusable for any configuration dialog that needs search capabilities.

## Current Implementation Details

### Files to Modify
- `/apps/frontend/src/routes/repositories/index.tsx` - Repository list page (will add Configure button)
- `/apps/frontend/src/routes/repositories/$repositoryId.tsx` - Current individual edit page (will be simplified)
- `/apps/frontend/src/features/repositories/components/repository-form.tsx` - General settings
- `/apps/frontend/src/features/repositories/components/code-execution-config.tsx` - Code execution settings
- `/apps/frontend/src/features/repositories/components/preview-config.tsx` - Preview environment settings

### Current Flow
1. User navigates to `/repositories` to see list of repositories
2. User clicks on a repository to go to `/repositories/$repositoryId`
3. User sees three configuration cards stacked vertically
4. Each card has its own save button

### New Flow
1. User navigates to `/repositories` to see list of repositories
2. User clicks "Configure Repositories" button to open dialog
3. Dialog shows all repositories in sidebar with search box
4. Clicking a repository shows all its configuration in one view

## Implementation Instructions

### Step 1: Extend ConfigurationDialog with Search Support

Modify `/apps/frontend/src/lib/components/configuration/configuration-dialog.tsx` to add optional search functionality:

```typescript
// Add to imports
import { Search } from 'lucide-react'
import { Input } from '../ui/input'

// Update ConfigurationDialogProps interface
export interface ConfigurationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  sections: ConfigurationSection[]
  activeSection: string
  onSectionChange: (sectionId: string) => void
  className?: string
  // New search props
  searchable?: boolean
  searchPlaceholder?: string
  onSearchChange?: (query: string) => void
  searchValue?: string
  // Optional custom sidebar footer
  sidebarFooter?: ReactNode
}

// Update ConfigurationDialog component
export function ConfigurationDialog({
  // ... existing props
  searchable,
  searchPlaceholder = "Search...",
  onSearchChange,
  searchValue = "",
  sidebarFooter,
}: ConfigurationDialogProps) {
  // ... existing code ...

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent /* ... existing props ... */>
        <div className="flex h-full">
          {/* Sidebar */}
          <div className="w-64 bg-muted/30 border-r flex flex-col">
            {/* Header - existing code */}
            
            {/* Add Search Box after header if searchable */}
            {searchable && (
              <div className="p-4 border-b">
                <div className="relative">
                  <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                  <Input
                    placeholder={searchPlaceholder}
                    value={searchValue}
                    onChange={(e) => onSearchChange?.(e.target.value)}
                    className="pl-9"
                  />
                </div>
              </div>
            )}

            {/* Navigation - existing code */}
            <nav className="flex-1 p-4">
              {sections.length === 0 ? (
                <div className="text-center text-sm text-muted-foreground py-4">
                  {searchValue ? 'No results found' : 'No sections available'}
                </div>
              ) : (
                <ul className="space-y-2">
                  {/* ... existing section mapping ... */}
                </ul>
              )}
            </nav>
            
            {/* Optional sidebar footer */}
            {sidebarFooter && (
              <div className="p-4 border-t">
                {sidebarFooter}
              </div>
            )}
          </div>

          {/* Content Area - existing code */}
        </div>
      </DialogContent>
    </Dialog>
  )
}

// Update MobileConfigurationDialog similarly
export function MobileConfigurationDialog({
  // ... existing props
  searchable,
  searchPlaceholder = "Search...",
  onSearchChange,
  searchValue = "",
}: ConfigurationDialogProps) {
  // Add search box in mobile header area
  // Similar implementation but adapted for mobile layout
}
```

### Step 2: Create Repositories Configuration Dialog

Create `/apps/frontend/src/features/repositories/components/repositories-configuration-dialog.tsx`:

```typescript
import { useState, useMemo, useEffect } from 'react'
import { FolderGit2, Plus } from 'lucide-react'
import { useNavigate } from '@tanstack/react-router'
import { Button } from '@/lib/components/ui/button'
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration'
import { useRepositoriesList } from '../hooks/use-repositories'
import { RepositoryAllConfigSection } from './sections/repository-all-config-section'

interface RepositoriesConfigurationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialRepositoryId?: string
}

export function RepositoriesConfigurationDialog({
  open,
  onOpenChange,
  initialRepositoryId,
}: RepositoriesConfigurationDialogProps) {
  const navigate = useNavigate()
  const { data: repositories = [], isLoading } = useRepositoriesList()
  const [searchQuery, setSearchQuery] = useState('')
  const [activeSection, setActiveSection] = useState(initialRepositoryId || '')

  // Set initial active section when repositories load
  useEffect(() => {
    if (!activeSection && repositories.length > 0 && !initialRepositoryId) {
      setActiveSection(repositories[0].id)
    }
  }, [repositories, activeSection, initialRepositoryId])

  // Filter and map repositories to sections
  const sections = useMemo(() => {
    const filtered = searchQuery 
      ? repositories.filter(repo => 
          repo.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
          repo.path.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : repositories

    return filtered.map(repo => ({
      id: repo.id,
      label: repo.name,
      icon: <FolderGit2 className="h-4 w-4" />,
      component: <RepositoryAllConfigSection repositoryId={repo.id} />
    }))
  }, [repositories, searchQuery])

  const handleAddRepository = () => {
    onOpenChange(false)
    navigate({ to: '/repositories/new' })
  }

  // Create sidebar footer with Add Repository button
  const sidebarFooter = (
    <Button className="w-full" size="sm" onClick={handleAddRepository}>
      <Plus className="h-4 w-4 mr-2" />
      Add Repository
    </Button>
  )

  if (isLoading && !open) {
    return null
  }

  return (
    <ResponsiveConfigurationDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Repositories"
      description="Configure your repositories"
      sections={sections}
      activeSection={activeSection}
      onSectionChange={setActiveSection}
      searchable
      searchPlaceholder="Search repositories..."
      searchValue={searchQuery}
      onSearchChange={setSearchQuery}
      sidebarFooter={sidebarFooter}
      className="sm:max-w-[1400px]" // Wider for repository configuration
    />
  )
}
```

### Step 3: Create Repository All-Config Section

Create `/apps/frontend/src/features/repositories/components/sections/repository-all-config-section.tsx`:

This component combines all three configuration areas into one scrollable view:

```typescript
import { useMemo } from 'react'
import { toast } from 'sonner'
import type { UpdateRepositoryRequest } from '@moatless/types'
import { 
  ConfigurationSection, 
  ConfigurationGroup, 
  ConfigurationItem,
  ConfigurationInput,
  ConfigurationTextarea,
  ConfigurationSelect,
  ConfigurationSwitch
} from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { Badge } from '@/lib/components/ui/badge'
import { 
  useRepository, 
  useUpdateRepository,
  useDetectFramework,
  useValidateCodeExecution 
} from '@/features/repositories/hooks/use-repositories'
import { LocalRepositoryConfig } from '../local-repository-config'
import { getErrorMessage } from '@/lib/utils/error'

interface RepositoryAllConfigSectionProps {
  repositoryId: string
}

export function RepositoryAllConfigSection({ repositoryId }: RepositoryAllConfigSectionProps) {
  const { data: repository, isLoading, error } = useRepository(repositoryId)
  const updateRepository = useUpdateRepository()
  const detectFramework = useDetectFramework(repositoryId)
  const validateExecution = useValidateCodeExecution(repositoryId)

  // Debounced update handler
  const debouncedUpdate = useMemo(
    () => debounce((updates: UpdateRepositoryRequest) => {
      updateRepository.mutate(
        { repoId: repositoryId, request: updates },
        {
          onSuccess: () => toast.success('Repository updated'),
          onError: (error) => toast.error(getErrorMessage(error))
        }
      )
    }, 500),
    [repositoryId, updateRepository]
  )

  if (isLoading) {
    return <ConfigurationSkeleton />
  }

  if (error || !repository) {
    return (
      <div className="text-center text-destructive">
        Failed to load repository configuration
      </div>
    )
  }

  return (
    <ConfigurationSection>
      {/* General Settings */}
      <ConfigurationGroup title="General Settings">
        <ConfigurationItem 
          label="Repository Name" 
          description="Display name for your repository"
        >
          <ConfigurationInput
            value={repository.name}
            onChange={(e) => debouncedUpdate({ name: e.target.value })}
          />
        </ConfigurationItem>

        <ConfigurationItem 
          label="Description" 
          description="Optional description of this repository"
        >
          <ConfigurationTextarea
            value={repository.description || ''}
            onChange={(e) => debouncedUpdate({ description: e.target.value })}
            rows={3}
          />
        </ConfigurationItem>

        <ConfigurationItem 
          label="Repository Path" 
          description="Local path to your repository"
        >
          <ConfigurationInput
            value={repository.path}
            onChange={(e) => debouncedUpdate({ path: e.target.value })}
          />
        </ConfigurationItem>

        <ConfigurationItem 
          label="Git Worktrees" 
          description="Use git worktrees for isolated environments"
        >
          <ConfigurationSwitch
            checked={repository.useGitWorktrees || false}
            onCheckedChange={(checked) => debouncedUpdate({ useGitWorktrees: checked })}
          />
        </ConfigurationItem>
      </ConfigurationGroup>

      {/* Code Execution Settings */}
      <ConfigurationGroup title="Code Execution" showSeparator>
        <ConfigurationItem 
          label="Execution Type" 
          description="Choose how to run code in this repository"
        >
          <ConfigurationSelect
            value={repository.codeExecution?.type || 'local'}
            onValueChange={(value) => 
              debouncedUpdate({ 
                codeExecution: { ...repository.codeExecution, type: value as 'local' | 'docker' } 
              })
            }
            options={[
              { value: 'local', label: 'Local' },
              { value: 'docker', label: 'Docker' }
            ]}
          />
        </ConfigurationItem>

        {repository.codeExecution?.type === 'docker' && (
          <ConfigurationItem 
            label="Docker Image" 
            description="Docker image to use for code execution"
          >
            <ConfigurationInput
              value={repository.codeExecution.dockerImage || ''}
              onChange={(e) => 
                debouncedUpdate({
                  codeExecution: { 
                    ...repository.codeExecution, 
                    dockerImage: e.target.value 
                  }
                })
              }
              placeholder="e.g., node:20-alpine"
            />
          </ConfigurationItem>
        )}

        <ConfigurationItem 
          label="Setup Commands" 
          description="Commands to run before executing code"
        >
          <div className="space-y-2">
            <ConfigurationTextarea
              value={repository.codeExecution?.setupCommands?.join('\n') || ''}
              onChange={(e) => 
                debouncedUpdate({
                  codeExecution: { 
                    ...repository.codeExecution, 
                    setupCommands: e.target.value.split('\n').filter(Boolean)
                  }
                })
              }
              placeholder="npm install&#10;npm run build"
              rows={4}
            />
            <Button 
              size="sm" 
              variant="outline"
              onClick={() => validateExecution.mutate()}
              disabled={validateExecution.isPending}
            >
              {validateExecution.isPending ? 'Validating...' : 'Validate'}
            </Button>
          </div>
        </ConfigurationItem>

        {/* Environment Variables - keep existing component structure */}
        <ConfigurationItem 
          label="Environment Variables" 
          description="Variables available during code execution"
        >
          <EnvironmentVariablesList 
            variables={repository.codeExecution?.environmentVariables || {}}
            onChange={(vars) => 
              debouncedUpdate({
                codeExecution: { 
                  ...repository.codeExecution, 
                  environmentVariables: vars
                }
              })
            }
          />
        </ConfigurationItem>
      </ConfigurationGroup>

      {/* Preview Environment Settings */}
      <ConfigurationGroup title="Preview Environment" showSeparator>
        <ConfigurationItem 
          label="Framework" 
          description="Web framework used in this repository"
        >
          <div className="flex gap-2">
            <ConfigurationSelect
              value={repository.previewConfig?.framework || 'other'}
              onValueChange={(value) => 
                debouncedUpdate({
                  previewConfig: { ...repository.previewConfig, framework: value }
                })
              }
              options={[
                { value: 'nextjs', label: 'Next.js' },
                { value: 'react', label: 'React' },
                { value: 'vue', label: 'Vue' },
                { value: 'angular', label: 'Angular' },
                { value: 'svelte', label: 'Svelte' },
                { value: 'other', label: 'Other' }
              ]}
              className="flex-1"
            />
            <Button
              size="sm"
              variant="outline"
              onClick={async () => {
                const detected = await detectFramework.mutateAsync()
                if (detected) {
                  debouncedUpdate({ previewConfig: detected })
                }
              }}
              disabled={detectFramework.isPending}
            >
              {detectFramework.isPending ? 'Detecting...' : 'Auto-detect'}
            </Button>
          </div>
        </ConfigurationItem>

        <ConfigurationItem 
          label="Framework Version" 
          description="Version of the framework (optional)"
        >
          <ConfigurationInput
            value={repository.previewConfig?.frameworkVersion || ''}
            onChange={(e) => 
              debouncedUpdate({
                previewConfig: { 
                  ...repository.previewConfig, 
                  frameworkVersion: e.target.value 
                }
              })
            }
            placeholder="e.g., 14.0.0"
          />
        </ConfigurationItem>

        <ConfigurationItem 
          label="Start Command" 
          description="Command to start the preview server"
        >
          <ConfigurationInput
            value={repository.previewConfig?.startCommand || ''}
            onChange={(e) => 
              debouncedUpdate({
                previewConfig: { 
                  ...repository.previewConfig, 
                  startCommand: e.target.value 
                }
              })
            }
            placeholder="e.g., npm run dev"
          />
        </ConfigurationItem>

        <ConfigurationItem 
          label="Script Injection" 
          description="How to inject the preview script"
        >
          <ConfigurationSelect
            value={repository.previewConfig?.scriptInjection || 'html-injection'}
            onValueChange={(value) => 
              debouncedUpdate({
                previewConfig: { 
                  ...repository.previewConfig, 
                  scriptInjection: value as 'html-injection' | 'middleware'
                }
              })
            }
            options={[
              { value: 'html-injection', label: 'HTML Injection' },
              { value: 'middleware', label: 'Middleware' }
            ]}
          />
        </ConfigurationItem>

        {repository.previewConfig?.scriptInjection === 'html-injection' && (
          <ConfigurationItem 
            label="HTML File Path" 
            description="Path to the main HTML file"
          >
            <ConfigurationInput
              value={repository.previewConfig.htmlFilePath || ''}
              onChange={(e) => 
                debouncedUpdate({
                  previewConfig: { 
                    ...repository.previewConfig, 
                    htmlFilePath: e.target.value 
                  }
                })
              }
              placeholder="e.g., public/index.html"
            />
          </ConfigurationItem>
        )}

        {/* Collapsible Learn More sections */}
        <CollapsibleHelpSection title="Learn More">
          <p className="text-sm text-muted-foreground">
            The preview environment allows you to see changes in real-time...
          </p>
        </CollapsibleHelpSection>
      </ConfigurationGroup>
    </ConfigurationSection>
  )
}
```

Note: The `EnvironmentVariablesList` and `CollapsibleHelpSection` components should be extracted from the existing code and reused here.

### Step 4: Update Repository List Page

Modify `/apps/frontend/src/routes/repositories/index.tsx` to add the Configure button:

```typescript
import { useState } from 'react'
import { createFileRoute } from '@tanstack/react-router'
import { Settings } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { 
  RepositoryList, 
  RepositoriesConfigurationDialog,
  useRepositoriesList 
} from '@/features/repositories'

export const Route = createFileRoute('/repositories/')({
  component: RepositoriesIndex,
})

function RepositoriesIndex() {
  const { data: repositories, isLoading } = useRepositoriesList()
  const [configOpen, setConfigOpen] = useState(false)

  return (
    <div className="container mx-auto py-6">
      <div className="space-y-4">
        {/* Add Configure button to the header */}
        {repositories && repositories.length > 0 && (
          <div className="flex justify-end">
            <Button onClick={() => setConfigOpen(true)}>
              <Settings className="h-4 w-4 mr-2" />
              Configure Repositories
            </Button>
          </div>
        )}
        
        <RepositoryList repositories={repositories} isLoading={isLoading} />
        
        <RepositoriesConfigurationDialog
          open={configOpen}
          onOpenChange={setConfigOpen}
        />
      </div>
    </div>
  )
}
```

### Step 5: Simplify Individual Repository Route

Update `/apps/frontend/src/routes/repositories/$repositoryId.tsx` to open the dialog with specific repository selected:

```typescript
import { useNavigate } from '@tanstack/react-router'
import { createFileRoute } from '@tanstack/react-router'
import { Settings } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Badge } from '@/lib/components/ui/badge'
import { 
  RepositoriesConfigurationDialog,
  VerificationDialog,
  useRepository 
} from '@/features/repositories'

export const Route = createFileRoute('/repositories/$repositoryId')({
  component: RepositoryDetail,
})

function RepositoryDetail() {
  const navigate = useNavigate()
  const { repositoryId } = Route.useParams()
  const [configOpen, setConfigOpen] = useState(false)
  
  const { data: repository, isLoading, error } = useRepository(repositoryId)

  if (isLoading) {
    return // ... existing loading skeleton
  }

  if (error || !repository) {
    return // ... existing error state
  }

  const handleCloseConfig = (open: boolean) => {
    setConfigOpen(open)
    // Navigate back to list when dialog closes
    if (!open) {
      navigate({ to: '/repositories' })
    }
  }

  return (
    <div className="container mx-auto py-6">
      <div className="max-w-4xl mx-auto space-y-6">
        <div className="flex justify-between items-center">
          <div>
            <h1 className="text-2xl font-bold">{repository.name}</h1>
            {repository.description && (
              <p className="text-muted-foreground mt-1">{repository.description}</p>
            )}
          </div>
          <div className="flex gap-2">
            <VerificationDialog repositoryId={repositoryId} />
            <Button onClick={() => setConfigOpen(true)}>
              <Settings className="h-4 w-4 mr-2" />
              Configure
            </Button>
          </div>
        </div>
        
        {/* Simple summary cards */}
        <div className="grid gap-4 md:grid-cols-3">
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">Repository Path</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-sm text-muted-foreground">{repository.path}</p>
              {repository.useGitWorktrees && (
                <Badge variant="secondary" className="mt-2">Git Worktrees</Badge>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">Code Execution</CardTitle>
            </CardHeader>
            <CardContent>
              <Badge variant={repository.codeExecution?.type === 'docker' ? 'default' : 'secondary'}>
                {repository.codeExecution?.type || 'Not configured'}
              </Badge>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">Preview Environment</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-sm">
                {repository.previewConfig?.framework || 'Not configured'}
              </p>
            </CardContent>
          </Card>
        </div>
        
        <RepositoriesConfigurationDialog
          open={configOpen}
          onOpenChange={handleCloseConfig}
          initialRepositoryId={repositoryId}
        />
      </div>
    </div>
  )
}
```

### Step 6: Export New Components

Update `/apps/frontend/src/features/repositories/index.ts` to export:
- `RepositoriesConfigurationDialog`
- Remove exports for individual section components that are no longer needed

## Important Implementation Notes

1. **Extending ConfigurationDialog**: The base dialog component is extended with optional search functionality, making it reusable for any configuration dialog that needs search.

2. **Clean implementation**: By extending the existing component, we avoid duplicating dialog logic and maintain consistency with the pattern.

3. **Search props**: 
   - `searchable`: Boolean to enable search
   - `searchPlaceholder`: Customizable placeholder text
   - `onSearchChange`: Callback for search value changes
   - `searchValue`: Controlled search input value
   - `sidebarFooter`: Optional footer component (used for Add Repository button)

4. **Repository sections**: Each repository becomes a section with its own icon, dynamically filtered based on search.

5. **Wider dialog**: Using `className="sm:max-w-[1400px]"` to accommodate the full repository configuration.

6. **Debounced updates**: All field changes in RepositoryAllConfigSection are debounced by 500ms.

7. **Extract reusable components**: 
   - `EnvironmentVariablesList` from `code-execution-config.tsx`
   - `CollapsibleHelpSection` from `preview-config.tsx`
   - `ConfigurationSkeleton` (create new or use existing skeleton pattern)
   - `debounce` utility function

8. **Handle missing hooks**: If `useValidateCodeExecution` doesn't exist, create it or adapt from existing validation logic.

9. **Mobile support**: The extended MobileConfigurationDialog should also support search in the mobile header area.

## Benefits of This Approach

1. **Reusable search functionality**: The extended ConfigurationDialog can be used for any configuration that needs search
2. **Minimal custom code**: Leverages existing ResponsiveConfigurationDialog component
3. **Consistent with pattern**: Follows the established configuration dialog pattern
4. **Clean separation**: Search logic is handled by the parent, dialog just displays filtered sections
5. **Flexible**: The sidebarFooter prop allows custom actions like "Add Repository"

## Verification Checklist

After implementation, verify:
- [ ] Search filters repositories by name and path
- [ ] All repositories appear in sidebar with icon, name, and path
- [ ] Clicking repository shows all three config sections in one scrollable view
- [ ] Add Repository button navigates to new repository page
- [ ] Mobile view shows search box and horizontal tabs
- [ ] All existing functionality preserved (validation, auto-detect, etc.)
- [ ] Debounced updates work without excessive API calls
- [ ] Dialog opens with correct repository when navigating from detail page
- [ ] Empty states show appropriate messages
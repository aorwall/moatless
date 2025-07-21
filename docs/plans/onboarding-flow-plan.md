# Onboarding Flow Implementation Instructions

## Overview
Implement a first-time user onboarding flow that guides users through initial setup. The onboarding must configure Claude paths, detect Docker, set up directory paths, and allow repository selection from Claude projects.

## Requirements

### Page 1 - System Configuration
Implement a configuration page with the following sections:

1. **Claude Configuration**: Display Claude binary and config directory status. Allow manual path entry if not found.
2. **Docker Status**: Show Docker installation and running status. Set Docker as default execution method.
3. **Data Directory**: Let user specify main settings directory (default: `~/.moatless`)
4. **Worktree Directory**: Configure where git worktrees will be created (default: `~/.moatless/worktrees`)
5. **Repository Directory**: Configure where new repositories will be cloned (default: `~/.moatless/repositories`)

### Page 2 - Repository Selection
Implement a repository selection page that:

1. Reads all projects from Claude config using the existing `getSuggestedRepositories` API
2. Filters based on worktree settings (if disabled, only show git repositories)
3. Displays checkboxes for each repository with Select All/None functionality
4. Creates selected repositories in batch when user confirms

## Existing Components to Use

- **Settings Service** (`/apps/backend/services/settings.service.ts`): Use for managing settings and system status checks
- **Claude Service** (`/apps/backend/agents/claude-code/claude.service.ts`): Use `getClaudeProjectDirectories()` for reading projects
- **Repository Service** (`/apps/backend/services/repository.service.ts`): Use `getSuggestedRepositories()` for project suggestions
- **ResponsiveConfigurationDialog**: Use as the base dialog pattern
- **Configuration field components**: Use existing `ConfigurationInput`, `ConfigurationSelect`, etc.

## Implementation Tasks

### 1. Backend Implementation

#### 1.1 Update Settings Schema
In `/packages/types/src/schemas/settings.schemas.ts`, add to `ApplicationSettingsSchema`:
```typescript
onboardingCompleted: z.boolean().optional(),
worktreeDir: z.string().optional(),
repositoryDir: z.string().optional(),
```

#### 1.2 Create Onboarding Endpoints
In `/apps/backend/routes/settings.ts`, add:
- `GET /api/settings/needs-onboarding` - Returns `{ needsOnboarding: boolean }`
- `POST /api/settings/complete-onboarding` - Sets `onboardingCompleted: true`

#### 1.3 Update Settings Service
In `/apps/backend/services/settings.service.ts`:
- Add defaults for `worktreeDir` and `repositoryDir` in constructor
- Add getter methods: `getWorktreeDir()`, `getRepositoryDir()`
- Ensure directories are created when settings are saved

#### 1.4 Create Batch Repository Endpoint
In `/apps/backend/routes/repositories.ts`, add:
- `POST /api/repositories/batch` - Accepts array of repository configs, creates all

### 2. Frontend Implementation

#### 2.1 Create File Structure
Create the following files:
```
/apps/frontend/src/features/onboarding/
├── components/
│   ├── onboarding-dialog.tsx
│   ├── system-setup-step.tsx
│   ├── repository-selection-step.tsx
│   └── onboarding-complete.tsx
├── hooks/
│   ├── use-onboarding-status.ts
│   └── use-repository-suggestions.ts
├── api/
│   └── onboarding.ts
└── index.ts
```

#### 2.2 Implement `onboarding-dialog.tsx`
Create a multi-step dialog using `ResponsiveConfigurationDialog`:
- Track current step state (1 or 2)
- Pass step components as sections
- Handle navigation between steps
- Call complete onboarding API on finish

#### 2.3 Implement `system-setup-step.tsx`
Create form sections using existing configuration components:

```typescript
// Use these components from @/lib/components/configuration:
// - ConfigurationSection
// - ConfigurationGroup
// - ConfigurationItem
// - ConfigurationInput
// - ConfigurationSwitch

// Sections to implement:
// 1. Claude paths (use useSystemStatus hook)
// 2. Docker status (display only, from useSystemStatus)
// 3. Data directory input (with validation)
// 4. Worktree directory input (conditional on worktree enabled)
// 5. Repository directory input
```

#### 2.4 Implement `repository-selection-step.tsx`
```typescript
// Use useRepositorySuggestions hook to load projects
// Display as checkbox list with:
// - Repository name and path
// - Git status indicator
// - Select all/none buttons
// - Filter toggle for git repos only
```

### 3. App Entry Point Integration

In `/apps/frontend/src/routes/index.tsx`, add onboarding check:

```typescript
// Add to Dashboard component:
const { data: needsOnboarding } = useOnboardingStatus()
const [showOnboarding, setShowOnboarding] = useState(false)

useEffect(() => {
  if (needsOnboarding?.needsOnboarding) {
    setShowOnboarding(true)
  }
}, [needsOnboarding])

// Add before main UI:
if (showOnboarding) {
  return <OnboardingDialog open={true} onComplete={() => setShowOnboarding(false)} />
}
```

### 4. Update General Settings

In `/apps/frontend/src/features/settings/components/general-configuration-section.tsx`:
- Add worktreeDir field (show only if worktrees enabled)
- Add repositoryDir field
- Use same ConfigurationInput components as onboarding

### 5. API Implementation

Create `/apps/frontend/src/features/onboarding/api/onboarding.ts`:

```typescript
export const onboardingApi = {
  needsOnboarding: () => apiClient.get<{ needsOnboarding: boolean }>('/settings/needs-onboarding'),
  completeOnboarding: () => apiClient.post('/settings/complete-onboarding'),
  createRepositoriesBatch: (repositories: CreateRepositoryRequest[]) => 
    apiClient.post('/repositories/batch', { repositories })
}
```

### 6. Directory Structure Created

The implementation must ensure these directories are created:
```
~/.moatless/                    # Main data directory
├── settings.json               # Application settings
├── events/                     # Task events
├── repositories/               # Repository configurations
├── tasks/                      # Task metadata
├── workspaces/                 # Workspace data
└── worktrees/                  # Git worktrees (if enabled)

~/.moatless/repositories/       # Cloned repositories directory
```

### 7. Validation Requirements

- All directory paths must be absolute paths
- Directories must be writable by the application
- Claude binary must be executable if path is provided
- Repository paths must exist and be accessible
# Configuration Dialog Pattern

## Overview

The Configuration Dialog pattern provides a reusable, ChatGPT-style dialog interface for managing application configuration. It features a sidebar navigation for different configuration sections and a responsive design that adapts to mobile devices.

## Key Features

- **Sidebar Navigation**: ChatGPT-style left sidebar with section navigation
- **Responsive Design**: Automatically switches between desktop sidebar and mobile tabs
- **Reusable Interface**: Generic component that works for any configuration context
- **Type-Safe Sections**: Strongly typed section definitions with React components
- **Accessibility**: Proper ARIA labels, keyboard navigation, and screen reader support

## Architecture

### Component Structure

```
configuration/
├── configuration-dialog.tsx       # All dialog components
│   ├── ResponsiveConfigurationDialog  # Main wrapper (auto-detects mobile/desktop)
│   ├── ConfigurationDialog           # Desktop version (sidebar)
│   └── MobileConfigurationDialog     # Mobile version (tabs)
├── configuration-section.tsx      # Section wrapper component
├── configuration-item.tsx         # Two-column layout component
├── configuration-group.tsx        # Group component with separator
├── fields/                        # Field wrapper components
│   ├── configuration-input.tsx
│   ├── configuration-select.tsx
│   ├── configuration-switch.tsx
│   └── configuration-textarea.tsx
└── index.ts                       # Barrel exports
```

### Section Definition

```typescript
export interface ConfigurationSection {
  id: string        // Unique identifier for the section
  label: string     // Display name in navigation
  icon?: ReactNode  // Optional icon for the section
  component: ReactNode // React component to render for this section
}
```

## Implementation Examples

### Basic Usage

```typescript
import { useState } from 'react'
import { Settings, Code, Database } from 'lucide-react'
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration'
import { GeneralSettingsSection } from './sections/general-settings-section'
import { ClaudeCodeSettingsSection } from './sections/claude-code-settings-section'

function SettingsDialog() {
  const [open, setOpen] = useState(false)
  const [activeSection, setActiveSection] = useState('general')

  const sections = [
    {
      id: 'general',
      label: 'General',
      icon: <Settings className="h-4 w-4" />,
      component: <GeneralSettingsSection />, // Self-contained with own save/discard
    },
    {
      id: 'claude-code',
      label: 'Claude Code',
      icon: <Code className="h-4 w-4" />,
      component: <ClaudeCodeSettingsSection />, // Self-contained with own save/discard
    },
  ]

  return (
    <ResponsiveConfigurationDialog
      open={open}
      onOpenChange={setOpen}
      title="Settings"
      description="Configure your application preferences"
      sections={sections}
      activeSection={activeSection}
      onSectionChange={setActiveSection}
      // No footer props needed - each section handles its own save/discard
    />
  )
}
```

## ⚠️ Critical Implementation Patterns

### Stable Component Pattern (Prevents Re-render Issues)

**Problem**: Component recreation causing focus loss and form submission issues.

When implementing dynamic sections (search/filter), avoid creating component instances in render loops. This causes React to unmount/remount components, leading to:
- Input fields losing focus while typing
- Form submission triggered on Enter key
- Component state being reset
- Performance degradation

**❌ Bad: Component recreation causes focus loss**
```typescript
const sections = useMemo(() => {
  const filtered = repositories.filter(repo => 
    repo.name.includes(searchQuery)
  )
  
  return filtered.map(repo => ({
    id: repo.id,
    label: repo.name,
    // This creates new component instances on every search!
    component: <RepositorySection repositoryId={repo.id} />
  }))
}, [repositories, searchQuery]) // searchQuery change = components recreated

// This also recreates components on every render
<ResponsiveConfigurationDialog
  sections={sections.map(section => ({
    ...section,
    component: section.id === activeSection ? renderActiveComponent() : null
  }))}
/>
```

**✅ Good: Stable Component Pattern**
```typescript
// 1. Create stable section metadata only
const stableSections = useMemo(() => {
  const filtered = repositories.filter(repo => 
    repo.name.includes(searchQuery)
  )
  
  return filtered.map(repo => ({
    id: repo.id,
    label: repo.name,
    icon: <FolderGit2 className="h-4 w-4" />,
    // Store metadata, not component instances
    componentType: 'repository' as const,
    repositoryId: repo.id,
  }))
}, [repositories, searchQuery])

// 2. Create stable component with proper memoization and keys
const activeComponent = useMemo(() => {
  const currentSection = stableSections.find(s => s.id === activeSection)
  if (!currentSection) return null
  
  if (currentSection.componentType === 'repository' && currentSection.repositoryId) {
    return (
      <RepositorySection
        key={currentSection.repositoryId} // Stable key prevents recreation
        repositoryId={currentSection.repositoryId}
        // Pass state management props for lifting state
        userModifications={currentRepositoryModifications}
        onUserModificationsChange={handleUserModificationsChange}
      />
    )
  }
  // Handle other component types...
  return null
}, [
  stableSections,
  activeSection,
  currentRepositoryModifications,
  handleUserModificationsChange,
  // Only include dependencies that actually matter
])

// 3. Pass stable references to dialog
<ResponsiveConfigurationDialog
  sections={stableSections.map(section => ({
    id: section.id,
    label: section.label,
    icon: section.icon,
    component: section.id === activeSection ? activeComponent : null,
  }))}
  // ... other props
/>
```

**Key Principles:**
1. **Separate Data from Components**: Store component metadata, not instances
2. **Stable Keys**: Use stable, meaningful keys (like IDs) for React reconciliation
3. **Proper Memoization**: Use `useMemo` with correct dependency arrays
4. **Avoid Function Calls in Render**: Don't call component-creating functions during render

### Form Submission Prevention Pattern

**Problem**: Enter key in input fields triggers unwanted form submission.

Configuration dialogs can have implicit form contexts that cause Enter key events to bubble up and trigger form submission, disrupting the user experience.

**✅ Solution: Comprehensive Form Prevention**

#### 1. Button Type Safety
```typescript
// ❌ Bad: Buttons without explicit type can submit forms
<Button onClick={onSave} disabled={!hasChanges}>Save</Button>

// ✅ Good: Explicit button type prevents form submission
<Button type="button" onClick={onSave} disabled={!hasChanges}>Save</Button>
```

#### 2. Input Field Protection
```typescript
// ConfigurationInput component with form prevention
export interface ConfigurationInputProps extends InputHTMLAttributes<HTMLInputElement> {
  allowFormSubmit?: boolean
}

export function ConfigurationInput({ 
  allowFormSubmit = false, 
  onKeyDown, 
  ...props 
}: ConfigurationInputProps) {
  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    // Prevent form submission on Enter unless explicitly allowed
    if (e.key === 'Enter' && !allowFormSubmit) {
      e.preventDefault()
    }
    
    // Call original onKeyDown if provided
    onKeyDown?.(e)
  }

  return <Input onKeyDown={handleKeyDown} {...props} />
}
```

#### 3. Textarea Field Protection
```typescript
export function ConfigurationTextarea({ 
  allowFormSubmit = false, 
  onKeyDown, 
  ...props 
}: ConfigurationTextareaProps) {
  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // For textareas, prevent Ctrl+Enter form submission
    // (regular Enter should still work for new lines)
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !allowFormSubmit) {
      e.preventDefault()
    }
    
    onKeyDown?.(e)
  }

  return <Textarea onKeyDown={handleKeyDown} {...props} />
}
```

#### 4. Dialog Footer Button Safety
```typescript
// Both desktop and mobile dialogs need explicit button types
<div className="flex justify-between gap-3">
  <Button
    type="button" // Prevents form submission
    variant="outline"
    onClick={onDiscard}
    disabled={!hasChanges || isSaving}
  >
    {discardButtonText}
  </Button>
  <Button
    type="button" // Prevents form submission
    onClick={onSave}
    disabled={!hasChanges || isSaving}
  >
    {saveButtonText}
  </Button>
</div>
```

### Self-Contained Section Pattern (Recommended)

**Problem**: Complex state lifting between dialog and sections causes issues with component recreation and state management.

**✅ Solution: Self-Contained Sections with Local Save/Discard**

Each configuration section manages its own state and provides its own save/discard buttons. This eliminates complex state lifting and makes each section independent.

#### Section Implementation
```typescript
export function RepositoryConfigSection({ repositoryId }: { repositoryId: string }) {
  const { data: repository } = useRepository(repositoryId)
  const updateRepository = useUpdateRepository()
  
  // Derive base form data from server state
  const baseFormData = useMemo(() => ({
    name: repository?.name || '',
    description: repository?.description || '',
    // ... other fields
  }), [repository])
  
  // Track user modifications locally
  const [userModifications, setUserModifications] = useState<Partial<UpdateRepositoryRequest>>({})
  
  // Merge server data with user modifications
  const formData = useMemo(() => ({
    ...baseFormData,
    ...userModifications,
  }), [baseFormData, userModifications])
  
  // Check for unsaved changes
  const hasChanges = Object.keys(userModifications).length > 0
  
  // Update specific field
  const updateField = (field: string, value: any) => {
    setUserModifications({
      ...userModifications,
      [field]: value,
    })
  }
  
  // Handle save
  const handleSave = () => {
    if (!hasChanges) return
    
    updateRepository.mutate(
      { repoId: repositoryId, request: userModifications },
      {
        onSuccess: () => {
          setUserModifications({})
          toast.success('Repository updated successfully')
        },
        onError: (error) => {
          toast.error('Failed to update repository', {
            description: getErrorMessage(error),
          })
        }
      }
    )
  }
  
  return (
    <ConfigurationSection>
      <ConfigurationInput
        value={formData.name || ''}
        onChange={(e) => updateField('name', e.target.value)}
      />
      {/* More form fields */}
      
      {/* Save/Discard buttons at section level */}
      <div className="flex justify-end gap-3 pt-6 border-t">
        <Button
          type="button"
          variant="outline"
          onClick={() => setUserModifications({})}
          disabled={!hasChanges}
        >
          Discard Changes
        </Button>
        <Button
          type="button"
          onClick={handleSave}
          disabled={!hasChanges || updateRepository.isPending}
        >
          {updateRepository.isPending ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Saving...
            </>
          ) : (
            <>
              <Save className="mr-2 h-4 w-4" />
              Save Changes
            </>
          )}
        </Button>
      </div>
    </ConfigurationSection>
  )
}
```

**Key Benefits:**
- **Simpler Implementation**: No complex state lifting or synchronization
- **Better Isolation**: Each section is completely independent
- **Easier Testing**: Test sections in isolation without dialog context
- **No State Tracking Issues**: Eliminates activeSection tracking problems
- **Clear User Intent**: Save button is right next to the fields being edited

### ⚠️ Never Use Form Elements in Configuration Dialogs

**Problem**: `<form>` elements can cause unwanted form submission behavior when Enter is pressed.

**❌ Bad: Form element causes submission issues**
```typescript
// This will submit the form when Enter is pressed in inputs
<form onSubmit={(e) => { e.preventDefault(); handleAdd() }}>
  <Input value={key} onChange={(e) => setKey(e.target.value)} />
  <Input value={value} onChange={(e) => setValue(e.target.value)} />
  <Button type="submit">Add</Button>
</form>
```

**✅ Good: Use div with explicit button handlers**
```typescript
<div className="flex items-center gap-2">
  <Input
    value={key}
    onChange={(e) => setKey(e.target.value)}
    onKeyDown={(e) => {
      if (e.key === 'Enter' && key && value) {
        e.preventDefault()
        handleAdd()
      }
    }}
  />
  <Input
    value={value}
    onChange={(e) => setValue(e.target.value)}
    onKeyDown={(e) => {
      if (e.key === 'Enter' && key && value) {
        e.preventDefault()
        handleAdd()
      }
    }}
  />
  <Button type="button" onClick={handleAdd} disabled={!(key && value)}>
    Add
  </Button>
</div>
```

### Repository Configuration Example

```typescript
const repositoryConfigSections = [
  {
    id: 'general',
    label: 'General',
    icon: <Settings className="h-4 w-4" />,
    component: <RepositoryGeneralSection repositoryId={repositoryId} />,
  },
  {
    id: 'git',
    label: 'Git Settings',
    icon: <GitBranch className="h-4 w-4" />,
    component: <RepositoryGitSection repositoryId={repositoryId} />,
  },
  {
    id: 'environments',
    label: 'Environments',
    icon: <Database className="h-4 w-4" />,
    component: <RepositoryEnvironmentsSection repositoryId={repositoryId} />,
  },
]
```

## Section Component Pattern

Each configuration section should be a self-contained component that handles its own state management and API interactions.

### Section Component Structure

```typescript
import { 
  ConfigurationSection,
  ConfigurationItem,
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationSelect,
  ConfigurationSwitch
} from '@/lib/components/configuration'

interface SectionProps {
  // Any props needed for the section (e.g., IDs, initial data)
}

export function ExampleConfigurationSection({ }: SectionProps) {
  // 1. Data fetching with React Query
  const { data, isLoading, error } = useConfigurationData()
  
  // 2. Mutation hooks for updates
  const updateSetting = useUpdateSetting()
  
  // 3. Direct event handlers (no useEffect needed)
  const handleChange = (field: string, value: any) => {
    updateSetting.mutate({ [field]: value })
  }

  // 4. Error and loading handling
  if (isLoading) return <ConfigurationSkeleton />
  if (error) return <ErrorAlert error={error} />

  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Basic Settings">
        <ConfigurationItem
          label="Project Name"
          description="The display name for your project"
        >
          <ConfigurationInput
            value={data.name}
            onChange={(e) => handleChange('name', e.target.value)}
          />
        </ConfigurationItem>
      </ConfigurationGroup>
    </ConfigurationSection>
  )
}
```

## Reusable Configuration Components

The configuration dialog pattern includes a set of reusable components for building consistent configuration sections:

### ConfigurationSection
Base wrapper for configuration sections:
```typescript
import { ConfigurationSection } from '@/lib/components/configuration'

<ConfigurationSection className="custom-spacing">
  {/* Configuration items */}
</ConfigurationSection>
```

### ConfigurationItem
Two-column layout component for individual settings:
```typescript
<ConfigurationItem
  label="Setting Name"
  description="Detailed description of this setting"
  htmlFor="custom-id" // Optional, auto-generated if not provided
>
  <Input id="custom-id" />
</ConfigurationItem>
```

### ConfigurationGroup
Groups related configuration items with optional title and separator:
```typescript
<ConfigurationGroup title="Advanced Settings" showSeparator={true}>
  <ConfigurationItem label="Enable Feature" description="Turn this feature on or off">
    <ConfigurationSwitch />
  </ConfigurationItem>
</ConfigurationGroup>
```

### Field Components
Pre-styled field components that work seamlessly with ConfigurationItem:
- `ConfigurationInput`: Text input field
- `ConfigurationSelect`: Dropdown selection with options array
- `ConfigurationSwitch`: Toggle switch
- `ConfigurationTextarea`: Multi-line text input

#### Two-Column Layout Guidelines

1. **Grid Structure**: Use `grid-cols-1 md:grid-cols-2` for responsive behavior
2. **Gap Spacing**: Use `gap-4` for consistent spacing between columns
3. **Alignment**: Use `items-start` to align items at the top
4. **Label Column**: 
   - Contains the label and description
   - Uses `space-y-1` for tight spacing
   - Label uses `text-sm font-medium`
   - Description uses `text-sm text-muted-foreground`
5. **Value Column**: 
   - Contains the input control (Input, Select, Textarea, etc.)
   - May include additional elements like buttons or badges
6. **Separators**: Use between logical groups of settings
7. **No Cards**: Avoid wrapping individual settings in cards; use clean rows with separators instead

## Responsive Behavior

### Desktop (≥640px)
- **Sidebar Navigation**: Fixed left sidebar with section list
- **Content Area**: Right side with scrollable content
- **Layout**: Two-column layout with 256px sidebar width

### Mobile (<640px)
- **Tab Navigation**: Horizontal scrollable tabs at the top
- **Content Area**: Full-width content below tabs
- **Layout**: Single-column stacked layout

### Responsive Implementation

```typescript
// Automatic responsive switching
<ResponsiveConfigurationDialog {...props} />

// Manual control (if needed)
<div className="hidden sm:block">
  <ConfigurationDialog {...props} />
</div>
<div className="sm:hidden">
  <MobileConfigurationDialog {...props} />
</div>
```

## State Management

### Dialog State
- `open`: Boolean controlling dialog visibility
- `activeSection`: String ID of currently selected section
- `onSectionChange`: Callback to handle section navigation

### Section State
Each section manages its own state independently:
- **Form data**: Local state for user inputs
- **API state**: React Query hooks for server state
- **UI state**: Loading, error, and validation states

## Component Guidelines

### Using Reusable Components

1. **Always use ConfigurationSection** as the root wrapper
2. **Use ConfigurationItem** for all settings to ensure consistent layout
3. **Group related settings** with ConfigurationGroup
4. **Leverage field components** instead of raw shadcn/ui components

### Styling Compliance

All configuration components follow the styling guide:
- Use semantic color tokens only
- No hardcoded colors or dark: prefixes
- Automatic theme switching via CSS variables
- Built on shadcn/ui components

### State Management

Following the derived state pattern:
- No useEffect for state synchronization
- Direct event handlers for updates
- Calculate derived values during render
- Use React Query for server state

### Configuration Section with Derived State Pattern

When building configuration sections that need to track changes and provide save functionality:

```typescript
export function ConfigurationSection({ id }: { id: string }) {
  const { data: serverData } = useServerData(id)
  const updateMutation = useUpdateData()

  // Derive base form data from server data
  const baseFormData = useMemo(() => ({
    name: serverData?.name || '',
    description: serverData?.description || '',
    // ... other fields
  }), [serverData])

  // Track user modifications separately
  const [userModifications, setUserModifications] = useState<Partial<FormData>>({})

  // Merge server data with user changes
  const formData = useMemo(
    () => ({ ...baseFormData, ...userModifications }),
    [baseFormData, userModifications]
  )

  // Check for unsaved changes
  const hasChanges = Object.keys(userModifications).length > 0

  // Update specific field
  const updateField = (field: string, value: any) => {
    setUserModifications(prev => ({ ...prev, [field]: value }))
  }

  // Save changes
  const handleSave = () => {
    updateMutation.mutate(userModifications, {
      onSuccess: () => {
        setUserModifications({})
        toast.success('Changes saved')
      }
    })
  }

  return (
    <ConfigurationSection>
      <ConfigurationInput
        value={formData.name}
        onChange={(e) => updateField('name', e.target.value)}
      />
      {/* More fields */}
      
      <div className="flex justify-end gap-3 pt-6">
        <Button
          variant="outline"
          onClick={() => setUserModifications({})}
          disabled={!hasChanges}
        >
          Discard Changes
        </Button>
        <Button
          onClick={handleSave}
          disabled={!hasChanges || updateMutation.isPending}
        >
          Save Changes
        </Button>
      </div>
    </ConfigurationSection>
  )
}
```

This pattern provides:
- No auto-save behavior that can disrupt user experience
- Clear indication of unsaved changes
- Explicit save/discard actions
- Better performance (no debounced API calls on every keystroke)

## Styling and Theming

### CSS Classes
The dialog uses Tailwind CSS with semantic color tokens:

```typescript
// Active section styling
activeSection === section.id
  ? 'bg-primary/10 text-primary border border-primary/20'
  : 'text-muted-foreground hover:text-foreground'

// Dialog sizing
'max-w-6xl w-[90vw] h-[80vh] max-h-[800px]'

// Sidebar styling
'w-64 bg-muted/30 border-r'
```

### Customization
Override default styles using the `className` prop:

```typescript
<ResponsiveConfigurationDialog
  className="max-w-4xl"  // Custom max width
  // ...other props
/>
```

## Integration with Existing Routes

### Route Integration Pattern

```typescript
// In your route component
export function SettingsRoute() {
  const [showDialog, setShowDialog] = useState(false)
  
  return (
    <div>
      {/* Trigger button */}
      <Button onClick={() => setShowDialog(true)}>
        Open Settings
      </Button>
      
      {/* Configuration dialog */}
      <SettingsConfigurationDialog 
        open={showDialog}
        onOpenChange={setShowDialog}
      />
    </div>
  )
}
```

### Deep Linking Support

```typescript
// Support URL-based section navigation
const [activeSection, setActiveSection] = useState(
  searchParams.get('section') || 'general'
)

// Update URL when section changes
const handleSectionChange = (sectionId: string) => {
  setActiveSection(sectionId)
  setSearchParams({ section: sectionId })
}
```

## Best Practices

### 1. Section Independence
- Each section should be self-contained
- Avoid cross-section dependencies
- Use context sparingly and only when necessary

### 2. Error Handling
- Handle loading and error states in each section
- Provide fallback UI for failed sections
- Use consistent error display patterns

### 3. Performance
- Lazy load section components when possible
- Use React Query for efficient caching
- Implement proper cleanup for event listeners

### 4. Accessibility
- Ensure keyboard navigation works properly
- Provide proper ARIA labels
- Support screen readers with semantic markup

### 5. Mobile Optimization
- Test on various screen sizes
- Ensure touch targets are adequate (44px minimum)
- Optimize for thumb navigation

## Common Use Cases

1. **Application Settings**: General app configuration
2. **Repository Configuration**: Per-repository settings
3. **Environment Setup**: Development environment configuration
4. **User Preferences**: Profile and preference management
5. **Integration Settings**: Third-party service configuration

## Migration from Page-Based Settings

When migrating existing page-based settings:

1. **Extract sections**: Move related settings into section components
2. **Preserve state logic**: Keep existing state management patterns
3. **Update navigation**: Replace page routes with dialog sections
4. **Test thoroughly**: Ensure all functionality is preserved

## Related Patterns

- **[React Query Patterns](./react-query.md)**: For data fetching in sections
- **[Error Handling Pattern](./error-handling.md)**: For section error boundaries
- **[Derived State Pattern](./derived-state.md)**: For form state management
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
ConfigurationDialog/
├── ResponsiveConfigurationDialog  # Main wrapper (auto-detects mobile/desktop)
├── ConfigurationDialog           # Desktop version (sidebar)
├── MobileConfigurationDialog     # Mobile version (tabs)
└── ConfigurationSection          # Type definition for sections
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
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration-dialog'
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
      component: <GeneralSettingsSection />,
    },
    {
      id: 'claude-code',
      label: 'Claude Code',
      icon: <Code className="h-4 w-4" />,
      component: <ClaudeCodeSettingsSection />,
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
    />
  )
}
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
interface SectionProps {
  // Any props needed for the section (e.g., IDs, initial data)
}

export function ExampleConfigurationSection({ }: SectionProps) {
  // 1. Data fetching with React Query
  const { data, isLoading, error } = useConfigurationData()
  
  // 2. Mutation hooks for updates
  const updateSetting = useUpdateSetting()
  
  // 3. Local state for form handling
  const [formData, setFormData] = useState({})

  // 4. Error and loading handling
  if (isLoading) return <ConfigurationSkeleton />
  if (error) return <ErrorAlert error={error} />

  return (
    <div className="space-y-6">
      {/* Section content */}
      <Card>
        <CardHeader>
          <CardTitle>Section Title</CardTitle>
          <CardDescription>Section description</CardDescription>
        </CardHeader>
        <CardContent>
          {/* Form fields and controls */}
        </CardContent>
      </Card>
    </div>
  )
}
```

### Two-Column Layout Pattern

Configuration sections should use a two-column layout for better readability and organization, similar to ChatGPT's settings interface:

```typescript
export function ConfigurationSection() {
  return (
    <div className="space-y-6">
      {/* Configuration item with two-column layout */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
        {/* Left column: Label and description */}
        <div className="space-y-1">
          <Label htmlFor={inputId} className="text-sm font-medium">
            Setting Name
          </Label>
          <p className="text-sm text-muted-foreground">
            Detailed description of what this setting controls
          </p>
        </div>
        
        {/* Right column: Input control */}
        <div>
          <Input
            id={inputId}
            type="text"
            placeholder="Default value"
            value={formData.settingName}
            onChange={handleChange}
          />
        </div>
      </div>

      {/* Use Separator between groups of settings */}
      <Separator />

      {/* Another configuration item */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
        <div className="space-y-1">
          <Label htmlFor={selectId} className="text-sm font-medium">
            Mode Selection
          </Label>
          <p className="text-sm text-muted-foreground">
            Choose the operational mode
          </p>
        </div>
        <Select value={formData.mode} onValueChange={handleModeChange}>
          <SelectTrigger id={selectId}>
            <SelectValue placeholder="Select mode" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="option1">Option 1</SelectItem>
            <SelectItem value="option2">Option 2</SelectItem>
          </SelectContent>
        </Select>
      </div>
    </div>
  )
}
```

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
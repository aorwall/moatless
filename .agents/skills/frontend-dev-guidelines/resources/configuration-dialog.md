# Configuration Dialog Pattern

## Overview

Configuration dialogs provide a sidebar-based interface for managing application settings. This pattern uses the `SidebarDialog` compound component for consistent layout and responsive behavior.

**When to use this pattern:**
- Application-wide settings (user preferences, system configuration)
- Repository or project-specific configuration
- Multi-section forms with navigation
- Any configuration interface with 2+ logical sections

**Key Features:**
- Sidebar navigation on desktop (≥640px)
- Sheet overlay with hamburger menu on mobile (<640px)
- Compound component API for flexible composition
- Automatic responsive behavior
- Search and footer support

## SidebarDialog Component API

The `SidebarDialog` compound component provides the structure for configuration dialogs.

### Component Interface

```typescript
// Root wrapper
<SidebarDialog
  open={boolean}              // Dialog visibility state
  onOpenChange={(open) => {}} // Callback for open state changes
  className={string}          // Optional: Custom dialog sizing
>
  {/* Child components */}
</SidebarDialog>

// Sidebar section (contains navigation)
<SidebarDialog.Sidebar
  title={string}              // Sidebar header title
  description={string}        // Optional: Subtitle text
>
  {/* Navigation, search, footer */}
</SidebarDialog.Sidebar>

// Search input (optional)
<SidebarDialog.Search
  value={string}              // Controlled search value
  onChange={(value) => {}}    // Search change handler
  placeholder={string}        // Optional: Custom placeholder
/>

// Navigation list wrapper
<SidebarDialog.Nav
  emptyMessage={string}       // Optional: Message when no items
>
  {/* NavItem components */}
</SidebarDialog.Nav>

// Individual navigation item
<SidebarDialog.NavItem
  active={boolean}            // Whether this item is active
  onClick={() => {}}          // Click handler
  icon={ReactNode}            // Optional: Icon element
>
  {children}                  // Item label text
</SidebarDialog.NavItem>

// Footer section (optional)
<SidebarDialog.Footer>
  {/* Footer content (e.g., Add button) */}
</SidebarDialog.Footer>

// Content area (right side)
<SidebarDialog.Content
  title={string}              // Optional: Content header title
>
  {/* Section content */}
</SidebarDialog.Content>
```

### Component Rules

- **MUST** wrap all content in `<SidebarDialog>` root component
- **MUST** provide both `<SidebarDialog.Sidebar>` and `<SidebarDialog.Content>`
- **MUST** use `<SidebarDialog.Nav>` to wrap navigation items
- **SHOULD** use `<SidebarDialog.Search>` for filterable sections
- **SHOULD** use `<SidebarDialog.Footer>` for action buttons (e.g., "Add New")
- **MAY** customize dialog width with `className` prop (default: `sm:max-w-[1200px]`)

## Configuration Field Components

Use these field components inside `SidebarDialog.Content` for consistent form styling.

### Available Components

```typescript
import {
  ConfigurationGroup,         // Groups related settings
  ConfigurationItem,          // Two-column label + field layout
  ConfigurationInput,         // Text input field
  ConfigurationSelect,        // Dropdown selection
  ConfigurationSwitch,        // Toggle switch
  ConfigurationTextarea,      // Multi-line text input
  ConfigurationSectionWrapper // Section container
} from '@/lib/components/configuration'
```

### Layout Pattern

```typescript
<ConfigurationSectionWrapper>
  <ConfigurationGroup title="Section Name">
    <ConfigurationItem
      label="Setting Label"
      description="Detailed description of this setting"
    >
      <ConfigurationInput
        value={formData.fieldName}
        onChange={(e) => updateField('fieldName', e.target.value)}
      />
    </ConfigurationItem>
  </ConfigurationGroup>
</ConfigurationSectionWrapper>
```

**MUST** use `ConfigurationSectionWrapper` as root for section content
**MUST** use `ConfigurationItem` for all settings to ensure consistent two-column layout
**SHOULD** group related settings with `ConfigurationGroup` and a descriptive title

## Complete Implementation Example

This example shows a settings dialog with multiple sections (from `apps/frontend/src/features/settings/components/settings-dialog.tsx:20-110`):

```typescript
import { Bot, Settings, Github } from 'lucide-react'
import { useMemo, useState } from 'react'
import { SidebarDialog } from '@/lib/components/ui/sidebar-dialog'
import {
  GeneralConfigurationSection,
  ClaudeConfigurationSection,
  GitHubConfigurationSection
} from './sections'

interface SettingsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultSection?: string
}

export function SettingsDialog({
  open,
  onOpenChange,
  defaultSection = 'general',
}: SettingsDialogProps) {
  const [activeSection, setActiveSection] = useState(defaultSection)

  // Define sections metadata (id, label, icon only)
  const sections = useMemo(() => [
    {
      id: 'general',
      label: 'General',
      icon: <Settings className="h-4 w-4" />,
    },
    {
      id: 'claude',
      label: 'Claude',
      icon: <Bot className="h-4 w-4" />,
    },
    {
      id: 'github',
      label: 'GitHub',
      icon: <Github className="h-4 w-4" />,
    },
  ], [])

  return (
    <SidebarDialog open={open} onOpenChange={onOpenChange}>
      <SidebarDialog.Sidebar
        title="Settings"
        description="Configure your application preferences"
      >
        <SidebarDialog.Nav>
          {sections.map(({ id, label, icon }) => (
            <SidebarDialog.NavItem
              key={id}
              active={activeSection === id}
              onClick={() => setActiveSection(id)}
              icon={icon}
            >
              {label}
            </SidebarDialog.NavItem>
          ))}
        </SidebarDialog.Nav>
      </SidebarDialog.Sidebar>

      <SidebarDialog.Content title={sections.find((s) => s.id === activeSection)?.label}>
        {/* Conditional rendering with stable keys */}
        {activeSection === 'general' && <GeneralConfigurationSection key="general" />}
        {activeSection === 'claude' && <ClaudeConfigurationSection key="claude" />}
        {activeSection === 'github' && <GitHubConfigurationSection key="github" />}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
```

### Key Implementation Points

1. **Sections Array**: Store only metadata (id, label, icon), NOT component instances
2. **Conditional Rendering**: Use conditional rendering in `SidebarDialog.Content` with stable keys
3. **Active Section**: Track which section is active with local state
4. **Section Components**: Each section is a self-contained component that handles its own data and state

## Advanced Example: Search and Footer

This example shows a repository configuration dialog with search and add functionality (from `apps/frontend/src/features/repositories/components/repositories-configuration-dialog.tsx:71-150`):

```typescript
export function RepositoriesConfigurationDialog({
  open,
  onOpenChange,
}: Props) {
  const { data: repositories = [] } = useRepositoriesList()
  const [searchQuery, setSearchQuery] = useState('')
  const [activeSection, setActiveSection] = useState('')
  const [showNewRepository, setShowNewRepository] = useState(false)

  // Filter repositories based on search
  const stableSections = useMemo(() => {
    const filtered = searchQuery
      ? repositories.filter((repo) =>
          repo.name.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : repositories

    const repoSections = filtered.map((repo) => ({
      id: repo.id,
      label: repo.name,
      icon: <FolderGit2 className="h-4 w-4" />,
      repositoryId: repo.id, // Additional metadata for rendering
    }))

    // Add "new repository" section if active
    if (showNewRepository) {
      return [
        {
          id: 'new',
          label: 'New Repository',
          icon: <Plus className="h-4 w-4" />,
        },
        ...repoSections,
      ]
    }

    return repoSections
  }, [repositories, searchQuery, showNewRepository])

  const handleAddRepository = () => {
    setShowNewRepository(true)
    setActiveSection('new')
    setSearchQuery('') // Clear search when adding new
  }

  const currentSection = stableSections.find((s) => s.id === activeSection)

  return (
    <SidebarDialog open={open} onOpenChange={onOpenChange} className="sm:max-w-[1400px]">
      <SidebarDialog.Sidebar title="Repositories" description="Configure your repositories">
        {/* Search input */}
        <SidebarDialog.Search
          value={searchQuery}
          onChange={setSearchQuery}
          placeholder="Search repositories..."
        />

        {/* Navigation list */}
        <SidebarDialog.Nav>
          {stableSections.map(({ id, label, icon }) => (
            <SidebarDialog.NavItem
              key={id}
              active={activeSection === id}
              onClick={() => setActiveSection(id)}
              icon={icon}
            >
              {label}
            </SidebarDialog.NavItem>
          ))}
        </SidebarDialog.Nav>

        {/* Footer with add button */}
        <SidebarDialog.Footer>
          <Button className="w-full" size="sm" onClick={handleAddRepository}>
            <Plus className="h-4 w-4 mr-2" />
            Add Repository
          </Button>
        </SidebarDialog.Footer>
      </SidebarDialog.Sidebar>

      {/* Content area with conditional rendering */}
      <SidebarDialog.Content title={currentSection?.label}>
        {currentSection?.id === 'new' && (
          <RepositoryConfigCreateSection key="new-repository" />
        )}
        {currentSection?.repositoryId && (
          <RepositoryConfigEditSection
            key={currentSection.repositoryId}
            repositoryId={currentSection.repositoryId}
          />
        )}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
```

### Advanced Pattern Notes

- **Search Filtering**: Filter sections array based on search query using `useMemo`
- **Dynamic Sections**: Add/remove sections (e.g., "New Repository") based on state
- **Footer Actions**: Use `SidebarDialog.Footer` for persistent action buttons
- **Custom Width**: Use `className` prop to override default dialog width
- **Metadata Enrichment**: Store additional metadata (like `repositoryId`) in sections array for conditional rendering

## Responsive Design

The `SidebarDialog` component automatically adapts to screen size:

### Desktop (≥640px)
- Fixed sidebar navigation on the left (256px width)
- Content area on the right with scrollable content
- Close button in top-right corner

### Mobile (<640px)
- Full-screen dialog
- Sidebar hidden by default
- Hamburger menu button in content header opens sidebar as Sheet overlay
- Sidebar slides in from left when opened
- Auto-closes when navigation item is selected

**Implementation Note**: Responsive behavior is automatic. No additional code required.

## Best Practices

### Section Management

- **MUST** use stable keys when conditionally rendering sections (e.g., `key="general"` or `key={repositoryId}`)
- **MUST** store only metadata in sections array (id, label, icon, plus any data needed for conditional rendering)
- **NEVER** store component instances in sections array (causes recreation and performance issues)
- **SHOULD** use `useMemo` for sections array when it depends on dynamic data (search, filters)

### State Management

- **MUST** use local state (`useState`) to track active section
- **SHOULD** lift state to parent only when section selection needs to be controlled externally
- **SHOULD** use React Query hooks in section components for server state
- Derive form state during render — no effects (see `../SKILL.md`)

### Section Components

- **MUST** make each section self-contained with its own data fetching and state
- **SHOULD** handle save/discard actions within each section component
- **SHOULD** use configuration field components for consistent styling
- See [Data Fetching](./data-fetching.md) for data fetching in sections

### Performance

- **MUST** use conditional rendering with stable keys (NOT creating components in map functions)
- **SHOULD** use `useMemo` for filtered/computed sections arrays
- **MAY** lazy load section components for very large dialogs (use `React.lazy`)

## Related Patterns

- **[Data Fetching](./data-fetching.md)** - For data fetching in configuration sections
- **[Error Handling Pattern](./error-handling.md)** - For section error states and boundaries

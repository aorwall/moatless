# Configuration Components Refactoring Proposal

## Executive Summary

This document proposes a refactoring of the configuration dialog components to follow a composable, Radix UI-inspired design system approach. The current implementation, while functional, has tight coupling between layout, behavior, and structure, making it difficult to create custom configurations beyond the pre-defined patterns.

**Goal**: Create a flexible, composable configuration system that:
- Separates structure from styling
- Enables easy composition of custom layouts
- Provides unstyled primitives with sensible defaults
- Maintains backward compatibility through preset components
- Follows established design system patterns (Radix UI, Headless UI)

## Current Architecture Analysis

### Component Structure

```
configuration/
├── configuration-dialog-responsive.tsx   # Responsive wrapper
├── configuration-dialog-desktop.tsx      # Desktop layout (sidebar)
├── configuration-dialog-mobile.tsx       # Mobile layout (tabs)
├── configuration-section.tsx             # Section wrapper (spacing)
├── configuration-item.tsx                # Two-column layout
├── configuration-group.tsx               # Group with separator
└── fields/
    ├── configuration-input.tsx
    ├── configuration-select.tsx
    ├── configuration-switch.tsx
    └── configuration-textarea.tsx
```

### Current Usage Pattern

```typescript
// Consumer defines sections with components
<ResponsiveConfigurationDialog
  open={open}
  onOpenChange={onOpenChange}
  title="Settings"
  description="Configure your application preferences"
  sections={[
    {
      id: 'general',
      label: 'General',
      icon: <Settings />,
      component: <GeneralConfigurationSection />
    }
  ]}
  activeSection={activeSection}
  onSectionChange={setActiveSection}
/>

// Section components use layout helpers
function GeneralConfigurationSection() {
  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Basic Settings">
        <ConfigurationItem label="Name" description="Your project name">
          <ConfigurationInput value={name} onChange={...} />
        </ConfigurationItem>
      </ConfigurationGroup>
    </ConfigurationSection>
  )
}
```

### Identified Limitations

1. **Tight Coupling**: Dialog component dictates entire layout structure
2. **Limited Flexibility**: Hard to create custom layouts (e.g., three-column, card-based)
3. **Duplication**: Desktop and mobile dialogs share significant code
4. **Inflexible Structure**: Sidebar/tabs are the only navigation options
5. **Monolithic Props**: Many props passed to root dialog component
6. **Hard to Extend**: Adding new features requires modifying core components
7. **Not Truly Composable**: Can't easily mix and match different parts

## Proposed Architecture: Composable Configuration System

### Design Principles

Following Radix UI and Headless UI patterns:

1. **Unstyled Primitives**: Provide structure and behavior, not appearance
2. **Compound Components**: Related components that share context
3. **Composition Over Configuration**: Build complex UIs by composing simple parts
4. **Escape Hatches**: Easy to override and customize
5. **Polymorphic Components**: Can render as different elements
6. **Controlled + Uncontrolled**: Support both patterns
7. **Accessibility First**: Built-in ARIA and keyboard navigation

### Component Hierarchy

```
Configuration Root (Context Provider)
├── Configuration.Dialog (Base dialog wrapper)
│   ├── Configuration.Content (Main content area)
│   │   ├── Configuration.Header (Optional header)
│   │   ├── Configuration.Sidebar (Optional sidebar)
│   │   │   ├── Configuration.Nav (Navigation list)
│   │   │   │   ├── Configuration.NavItem
│   │   │   │   └── Configuration.NavGroup
│   │   │   ├── Configuration.Search (Optional search)
│   │   │   └── Configuration.Footer (Optional footer)
│   │   └── Configuration.Main (Content area)
│   │       ├── Configuration.Section
│   │       │   ├── Configuration.Group
│   │       │   └── Configuration.Field
│   │       │       ├── Configuration.Label
│   │       │       ├── Configuration.Description
│   │       │       └── Configuration.Control
│   │       └── Configuration.Actions (Save/Cancel)
│   └── Configuration.Trigger (Optional trigger)
└── Configuration.Tabs (Alternative to sidebar)
    ├── Configuration.TabsList
    └── Configuration.TabsContent
```

### Core Primitives

#### 1. Configuration Root

Context provider that manages shared state:

```typescript
interface ConfigurationContextValue {
  activeSection: string
  setActiveSection: (id: string) => void
  searchQuery: string
  setSearchQuery: (query: string) => void
}

function Configuration({
  children,
  defaultSection,
  activeSection: controlledSection,
  onSectionChange
}: ConfigurationProps) {
  // Controlled/uncontrolled pattern
  const [internalSection, setInternalSection] = useState(defaultSection)
  const activeSection = controlledSection ?? internalSection
  const setActiveSection = onSectionChange ?? setInternalSection

  const [searchQuery, setSearchQuery] = useState('')

  return (
    <ConfigurationContext.Provider
      value={{ activeSection, setActiveSection, searchQuery, setSearchQuery }}
    >
      {children}
    </ConfigurationContext.Provider>
  )
}
```

#### 2. Configuration.Dialog

Wrapper around shadcn Dialog:

```typescript
function ConfigurationDialog({
  children,
  open,
  onOpenChange,
  ...props
}: ConfigurationDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent {...props}>
        {children}
      </DialogContent>
    </Dialog>
  )
}
```

#### 3. Configuration.Sidebar

Flexible sidebar component:

```typescript
function ConfigurationSidebar({
  children,
  className,
  position = 'left' // or 'right'
}: ConfigurationSidebarProps) {
  return (
    <aside
      className={cn(
        'flex flex-col border-r',
        position === 'right' && 'border-l border-r-0',
        className
      )}
    >
      {children}
    </aside>
  )
}
```

#### 4. Configuration.Nav

Navigation list that uses context:

```typescript
function ConfigurationNav({
  children,
  orientation = 'vertical' // or 'horizontal'
}: ConfigurationNavProps) {
  return (
    <nav>
      <ul className={cn(
        orientation === 'vertical' ? 'flex-col' : 'flex-row',
        'flex gap-2'
      )}>
        {children}
      </ul>
    </nav>
  )
}
```

#### 5. Configuration.NavItem

Individual navigation item:

```typescript
function ConfigurationNavItem({
  id,
  children,
  icon,
  disabled,
  asChild = false
}: ConfigurationNavItemProps) {
  const { activeSection, setActiveSection } = useConfiguration()
  const isActive = activeSection === id

  const Comp = asChild ? Slot : 'button'

  return (
    <li>
      <Comp
        onClick={() => !disabled && setActiveSection(id)}
        data-active={isActive}
        disabled={disabled}
        className={cn(
          'w-full flex items-center gap-3 px-3 py-2 rounded-md',
          'hover:bg-muted/50 transition-colors',
          isActive && 'bg-primary/10 text-primary'
        )}
      >
        {icon && <span className="w-5 h-5">{icon}</span>}
        {children}
      </Comp>
    </li>
  )
}
```

#### 6. Configuration.Section

Content section tied to navigation:

```typescript
function ConfigurationSection({
  id,
  children,
  className
}: ConfigurationSectionProps) {
  const { activeSection } = useConfiguration()

  if (activeSection !== id) return null

  return (
    <div className={cn('space-y-6', className)}>
      {children}
    </div>
  )
}
```

#### 7. Configuration.Field

Field wrapper with label and description:

```typescript
function ConfigurationField({
  children,
  className
}: ConfigurationFieldProps) {
  return (
    <div className={cn(
      'grid grid-cols-1 md:grid-cols-[1fr,2fr] gap-4 items-start',
      className
    )}>
      {children}
    </div>
  )
}

function ConfigurationLabel({ children, htmlFor }: ConfigurationLabelProps) {
  return (
    <div className="space-y-1">
      <Label htmlFor={htmlFor}>{children}</Label>
    </div>
  )
}

function ConfigurationDescription({ children }: ConfigurationDescriptionProps) {
  return <p className="text-sm text-muted-foreground">{children}</p>
}

function ConfigurationControl({ children }: ConfigurationControlProps) {
  return <div>{children}</div>
}
```

### Preset Components (Backward Compatibility)

Pre-composed components that match current API:

```typescript
// Preset matching current ResponsiveConfigurationDialog
function ConfigurationDialogPreset({
  open,
  onOpenChange,
  title,
  description,
  sections,
  activeSection,
  onSectionChange,
  searchable,
  searchPlaceholder,
  sidebarFooter,
}: PresetProps) {
  return (
    <Configuration
      activeSection={activeSection}
      onSectionChange={onSectionChange}
    >
      <Configuration.Dialog open={open} onOpenChange={onOpenChange}>
        <Configuration.Content className="sm:max-w-[1200px] h-[85vh]">

          {/* Desktop: Sidebar Layout */}
          <div className="hidden sm:flex h-full">
            <Configuration.Sidebar>
              <Configuration.Header>
                <DialogTitle>{title}</DialogTitle>
                <DialogDescription>{description}</DialogDescription>
              </Configuration.Header>

              {searchable && (
                <Configuration.Search placeholder={searchPlaceholder} />
              )}

              <ScrollArea className="flex-1">
                <Configuration.Nav>
                  {sections.map(section => (
                    <Configuration.NavItem
                      key={section.id}
                      id={section.id}
                      icon={section.icon}
                    >
                      {section.label}
                    </Configuration.NavItem>
                  ))}
                </Configuration.Nav>
              </ScrollArea>

              {sidebarFooter && (
                <Configuration.Footer>{sidebarFooter}</Configuration.Footer>
              )}
            </Configuration.Sidebar>

            <Configuration.Main>
              {sections.map(section => (
                <Configuration.Section key={section.id} id={section.id}>
                  {section.component}
                </Configuration.Section>
              ))}
            </Configuration.Main>
          </div>

          {/* Mobile: Tabs Layout */}
          <div className="sm:hidden">
            <Configuration.Tabs>
              <Configuration.TabsList>
                {sections.map(section => (
                  <Configuration.TabsItem key={section.id} value={section.id}>
                    {section.icon}
                    {section.label}
                  </Configuration.TabsItem>
                ))}
              </Configuration.TabsList>

              {sections.map(section => (
                <Configuration.TabsContent key={section.id} value={section.id}>
                  {section.component}
                </Configuration.TabsContent>
              ))}
            </Configuration.Tabs>
          </div>

        </Configuration.Content>
      </Configuration.Dialog>
    </Configuration>
  )
}
```

## Usage Examples

### Example 1: Basic Settings Dialog (Using Presets)

Backward-compatible approach:

```typescript
function SettingsDialog() {
  const [open, setOpen] = useState(false)

  return (
    <ConfigurationDialogPreset
      open={open}
      onOpenChange={setOpen}
      title="Settings"
      sections={[
        {
          id: 'general',
          label: 'General',
          icon: <Settings />,
          component: <GeneralSection />
        }
      ]}
    />
  )
}

function GeneralSection() {
  return (
    <Configuration.Group title="Basic Settings">
      <Configuration.Field>
        <Configuration.Label htmlFor="name">
          Project Name
          <Configuration.Description>
            The display name for your project
          </Configuration.Description>
        </Configuration.Label>
        <Configuration.Control>
          <Input id="name" />
        </Configuration.Control>
      </Configuration.Field>
    </Configuration.Group>
  )
}
```

### Example 2: Custom Layout (Composable Approach)

Custom three-column layout:

```typescript
function CustomConfigurationDialog() {
  return (
    <Configuration defaultSection="general">
      <Configuration.Dialog open={open} onOpenChange={setOpen}>
        <Configuration.Content className="max-w-[1400px]">

          <div className="grid grid-cols-[200px,1fr,300px] h-full">
            {/* Left sidebar: Categories */}
            <Configuration.Sidebar position="left">
              <Configuration.Nav>
                <Configuration.NavGroup title="General">
                  <Configuration.NavItem id="profile" icon={<User />}>
                    Profile
                  </Configuration.NavItem>
                  <Configuration.NavItem id="account" icon={<Settings />}>
                    Account
                  </Configuration.NavItem>
                </Configuration.NavGroup>

                <Configuration.NavGroup title="Advanced">
                  <Configuration.NavItem id="security" icon={<Lock />}>
                    Security
                  </Configuration.NavItem>
                </Configuration.NavGroup>
              </Configuration.Nav>
            </Configuration.Sidebar>

            {/* Center: Main content */}
            <Configuration.Main>
              <Configuration.Section id="profile">
                <ProfileSettings />
              </Configuration.Section>

              <Configuration.Section id="account">
                <AccountSettings />
              </Configuration.Section>

              <Configuration.Section id="security">
                <SecuritySettings />
              </Configuration.Section>
            </Configuration.Main>

            {/* Right sidebar: Preview/Help */}
            <aside className="border-l p-6 bg-muted/10">
              <h3 className="font-semibold mb-2">Help</h3>
              <p className="text-sm text-muted-foreground">
                Context-sensitive help content
              </p>
            </aside>
          </div>

        </Configuration.Content>
      </Configuration.Dialog>
    </Configuration>
  )
}
```

### Example 3: Card-Based Layout

Grid of configuration cards:

```typescript
function CardBasedConfiguration() {
  return (
    <Configuration>
      <Configuration.Dialog open={open} onOpenChange={setOpen}>
        <Configuration.Content>

          <div className="p-6">
            <h1 className="text-2xl font-bold mb-6">Settings</h1>

            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
              {/* Each card is a clickable section */}
              <Configuration.NavItem id="general" asChild>
                <Card className="cursor-pointer hover:border-primary">
                  <CardHeader>
                    <Settings className="h-6 w-6 mb-2" />
                    <CardTitle>General</CardTitle>
                    <CardDescription>
                      Basic application settings
                    </CardDescription>
                  </CardHeader>
                </Card>
              </Configuration.NavItem>

              <Configuration.NavItem id="integrations" asChild>
                <Card className="cursor-pointer hover:border-primary">
                  <CardHeader>
                    <Plug className="h-6 w-6 mb-2" />
                    <CardTitle>Integrations</CardTitle>
                    <CardDescription>
                      Connect external services
                    </CardDescription>
                  </CardHeader>
                </Card>
              </Configuration.NavItem>
            </div>

            {/* Detailed view appears below */}
            <div className="mt-8">
              <Configuration.Section id="general">
                <GeneralSettings />
              </Configuration.Section>

              <Configuration.Section id="integrations">
                <IntegrationsSettings />
              </Configuration.Section>
            </div>
          </div>

        </Configuration.Content>
      </Configuration.Dialog>
    </Configuration>
  )
}
```

### Example 4: Wizard/Stepper Layout

Multi-step configuration:

```typescript
function WizardConfiguration() {
  const steps = ['profile', 'preferences', 'integrations', 'review']
  const [currentStepIndex, setCurrentStepIndex] = useState(0)

  return (
    <Configuration
      activeSection={steps[currentStepIndex]}
      onSectionChange={(id) => {
        const index = steps.indexOf(id)
        if (index !== -1) setCurrentStepIndex(index)
      }}
    >
      <Configuration.Dialog open={open} onOpenChange={setOpen}>
        <Configuration.Content>

          {/* Stepper header */}
          <div className="border-b p-6">
            <Configuration.Nav orientation="horizontal">
              {steps.map((step, index) => (
                <Configuration.NavItem
                  key={step}
                  id={step}
                  disabled={index > currentStepIndex}
                >
                  Step {index + 1}
                </Configuration.NavItem>
              ))}
            </Configuration.Nav>
          </div>

          {/* Step content */}
          <div className="p-6 flex-1">
            <Configuration.Section id="profile">
              <ProfileStep />
            </Configuration.Section>

            <Configuration.Section id="preferences">
              <PreferencesStep />
            </Configuration.Section>

            <Configuration.Section id="integrations">
              <IntegrationsStep />
            </Configuration.Section>

            <Configuration.Section id="review">
              <ReviewStep />
            </Configuration.Section>
          </div>

          {/* Navigation buttons */}
          <div className="border-t p-6 flex justify-between">
            <Button
              variant="outline"
              onClick={() => setCurrentStepIndex(i => Math.max(0, i - 1))}
              disabled={currentStepIndex === 0}
            >
              Previous
            </Button>

            <Button
              onClick={() => setCurrentStepIndex(i => Math.min(steps.length - 1, i + 1))}
              disabled={currentStepIndex === steps.length - 1}
            >
              Next
            </Button>
          </div>

        </Configuration.Content>
      </Configuration.Dialog>
    </Configuration>
  )
}
```

### Example 5: Repository Configuration (Current Pattern)

Migrating the existing repository configuration:

```typescript
function RepositoriesConfiguration() {
  const { data: repositories = [] } = useRepositoriesList()
  const [searchQuery, setSearchQuery] = useState('')

  const filteredRepos = useMemo(() =>
    repositories.filter(repo =>
      repo.name.toLowerCase().includes(searchQuery.toLowerCase())
    ),
    [repositories, searchQuery]
  )

  return (
    <Configuration defaultSection={repositories[0]?.id}>
      <Configuration.Dialog open={open} onOpenChange={setOpen}>
        <Configuration.Content className="sm:max-w-[1400px]">

          <div className="flex h-full">
            <Configuration.Sidebar>
              <Configuration.Header>
                <DialogTitle>Repositories</DialogTitle>
                <DialogDescription>
                  Configure your repositories
                </DialogDescription>
              </Configuration.Header>

              <Configuration.Search
                value={searchQuery}
                onChange={setSearchQuery}
                placeholder="Search repositories..."
              />

              <ScrollArea className="flex-1">
                <Configuration.Nav>
                  {filteredRepos.map(repo => (
                    <Configuration.NavItem
                      key={repo.id}
                      id={repo.id}
                      icon={<FolderGit2 />}
                    >
                      {repo.name}
                    </Configuration.NavItem>
                  ))}
                </Configuration.Nav>
              </ScrollArea>

              <Configuration.Footer>
                <Button className="w-full" onClick={handleAddRepository}>
                  <Plus className="mr-2" />
                  Add Repository
                </Button>
              </Configuration.Footer>
            </Configuration.Sidebar>

            <Configuration.Main>
              {filteredRepos.map(repo => (
                <Configuration.Section key={repo.id} id={repo.id}>
                  <RepositoryEditSection repositoryId={repo.id} />
                </Configuration.Section>
              ))}
            </Configuration.Main>
          </div>

        </Configuration.Content>
      </Configuration.Dialog>
    </Configuration>
  )
}
```

## Migration Strategy

### Phase 1: Create Composable Primitives (Non-Breaking)

1. Create new composable components in parallel:
   ```
   configuration-v2/
   ├── configuration.tsx              # Root + context
   ├── configuration-dialog.tsx       # Dialog wrapper
   ├── configuration-content.tsx      # Content area
   ├── configuration-sidebar.tsx      # Sidebar
   ├── configuration-nav.tsx          # Navigation
   ├── configuration-section.tsx      # Section (new)
   ├── configuration-field.tsx        # Field components
   └── presets/
       └── configuration-dialog-preset.tsx  # Backward compat
   ```

2. Keep existing components unchanged
3. Update documentation with new patterns
4. Create migration guide

### Phase 2: Migrate Existing Usage

1. Start with simple dialogs (Settings)
2. Move to complex dialogs (Repositories)
3. Update tests
4. Document learnings

### Phase 3: Deprecate Old Components

1. Mark old components as deprecated
2. Provide migration path in JSDoc
3. Update all internal usage
4. Remove deprecated components in next major version

### Migration Example

**Before (Current)**:
```typescript
<ResponsiveConfigurationDialog
  open={open}
  onOpenChange={setOpen}
  title="Settings"
  sections={sections}
  activeSection={active}
  onSectionChange={setActive}
/>
```

**After (Composable)**:
```typescript
// Option 1: Use preset for backward compatibility
<ConfigurationDialogPreset
  open={open}
  onOpenChange={setOpen}
  title="Settings"
  sections={sections}
  activeSection={active}
  onSectionChange={setActive}
/>

// Option 2: Use composable API for more control
<Configuration activeSection={active} onSectionChange={setActive}>
  <Configuration.Dialog open={open} onOpenChange={setOpen}>
    <Configuration.Content>
      <Configuration.Sidebar>
        <Configuration.Nav>
          {sections.map(s => (
            <Configuration.NavItem key={s.id} id={s.id}>
              {s.label}
            </Configuration.NavItem>
          ))}
        </Configuration.Nav>
      </Configuration.Sidebar>

      <Configuration.Main>
        {sections.map(s => (
          <Configuration.Section key={s.id} id={s.id}>
            {s.component}
          </Configuration.Section>
        ))}
      </Configuration.Main>
    </Configuration.Content>
  </Configuration.Dialog>
</Configuration>
```

## Benefits of Refactoring

### 1. Flexibility
- Create custom layouts without modifying core components
- Mix and match different navigation patterns
- Easy to add new behaviors

### 2. Maintainability
- Smaller, focused components
- Clear separation of concerns
- Easier to test and debug

### 3. Developer Experience
- Intuitive composition API
- Type-safe with TypeScript
- Excellent autocomplete support

### 4. Consistency
- Follows established design system patterns
- Matches Radix UI mental model
- Easier onboarding for developers familiar with modern component libraries

### 5. Extensibility
- Easy to add new features
- Compose with other libraries
- Create custom variants without forking

### 6. Performance
- Smaller bundle size (tree-shakeable)
- Render only what's needed
- Better React optimization

## Implementation Checklist

### Core Primitives
- [ ] `Configuration` (Root + Context)
- [ ] `Configuration.Dialog`
- [ ] `Configuration.Content`
- [ ] `Configuration.Header`
- [ ] `Configuration.Sidebar`
- [ ] `Configuration.Nav` + `NavItem` + `NavGroup`
- [ ] `Configuration.Search`
- [ ] `Configuration.Footer`
- [ ] `Configuration.Main`
- [ ] `Configuration.Section`
- [ ] `Configuration.Group`
- [ ] `Configuration.Field` + `Label` + `Description` + `Control`
- [ ] `Configuration.Actions`
- [ ] `Configuration.Tabs` + `TabsList` + `TabsContent`

### Field Components
- [ ] Keep existing field components (Input, Select, etc.)
- [ ] Add new variants if needed

### Preset Components
- [ ] `ConfigurationDialogPreset` (matches ResponsiveConfigurationDialog)
- [ ] Desktop layout preset
- [ ] Mobile layout preset

### Documentation
- [ ] API reference for all components
- [ ] Migration guide
- [ ] Usage examples
- [ ] Best practices

### Testing
- [ ] Unit tests for primitives
- [ ] Integration tests for presets
- [ ] Accessibility tests
- [ ] Migration tests (old API still works)

### Migration
- [ ] Update SettingsDialog
- [ ] Update RepositoriesConfigurationDialog
- [ ] Update documentation
- [ ] Deprecate old components

## Conclusion

This refactoring proposal provides a path to a more flexible, maintainable configuration system while maintaining backward compatibility. The composable approach enables:

- **Custom layouts** without modifying core components
- **Better developer experience** with intuitive composition
- **Easier maintenance** through separation of concerns
- **Future-proof architecture** that's easy to extend

The preset components ensure existing code continues to work, while new features can leverage the full power of the composable API.

---

**Recommended Next Steps**:
1. Review and gather feedback on this proposal
2. Create proof-of-concept for core primitives
3. Implement Phase 1 (composable primitives)
4. Migrate one complex dialog (e.g., Repositories)
5. Evaluate and iterate based on learnings
6. Complete migration and deprecate old components

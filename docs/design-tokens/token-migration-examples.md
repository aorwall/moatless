# Token Migration Examples

This document provides concrete before/after examples for migrating components to the Three-Tier Design Token Architecture.

## Component Migration Examples

### 1. Button Component

**Before (Current Implementation):**
```tsx
// button.tsx
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]",
  {
    variants: {
      variant: {
        default: 'bg-primary text-primary-foreground shadow-xs hover:bg-primary/90',
        destructive: 'bg-destructive text-white shadow-xs hover:bg-destructive/90',
        outline: 'border bg-background shadow-xs hover:bg-accent hover:text-accent-foreground',
        secondary: 'bg-secondary text-secondary-foreground shadow-xs hover:bg-secondary/80',
        ghost: 'hover:bg-accent hover:text-accent-foreground',
        link: 'text-primary underline-offset-4 hover:underline',
      },
      size: {
        default: 'h-9 px-4 py-2 has-[>svg]:px-3',
        sm: 'h-8 rounded-md gap-1.5 px-3 has-[>svg]:px-2.5',
        lg: 'h-10 rounded-md px-6 has-[>svg]:px-4',
        icon: 'size-9',
      },
    },
  },
)
```

**After (With Design Tokens):**
```tsx
// button.tsx
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-[var(--semantic-spacing-sm)] whitespace-nowrap rounded-[var(--button-radius)] text-[length:var(--button-font-size)] font-[var(--button-font-weight)] transition-all duration-[var(--core-duration-fast)] disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none focus-visible:ring-[length:var(--focus-ring-width)] focus-visible:ring-[color:var(--focus-ring-color)] focus-visible:ring-opacity-[var(--focus-ring-opacity)]",
  {
    variants: {
      variant: {
        default: 'bg-[var(--button-primary-bg)] text-[var(--button-primary-text)] shadow-[var(--semantic-shadow-sm)] hover:bg-[var(--button-primary-bg-hover)]',
        destructive: 'bg-[var(--button-destructive-bg)] text-[var(--button-destructive-text)] shadow-[var(--semantic-shadow-sm)] hover:bg-[var(--button-destructive-bg-hover)]',
        outline: 'border border-[var(--semantic-border-primary)] bg-transparent shadow-[var(--semantic-shadow-sm)] hover:bg-[var(--semantic-accent)] hover:text-[var(--semantic-text-primary)]',
        secondary: 'bg-[var(--button-secondary-bg)] text-[var(--button-secondary-text)] shadow-[var(--semantic-shadow-sm)] hover:bg-[var(--button-secondary-bg-hover)]',
        ghost: 'hover:bg-[var(--semantic-accent)] hover:text-[var(--semantic-text-primary)]',
        link: 'text-[var(--semantic-brand-primary)] underline-offset-4 hover:underline',
      },
      size: {
        default: 'h-[var(--button-height-default)] px-[var(--button-padding-x-default)]',
        sm: 'h-[var(--button-height-sm)] gap-[calc(var(--semantic-spacing-sm)*0.75)] px-[var(--button-padding-x-sm)]',
        lg: 'h-[var(--button-height-lg)] px-[var(--button-padding-x-lg)]',
        icon: 'size-[var(--button-height-default)]',
      },
    },
  },
)
```

### 2. Card Component

**Before:**
```tsx
// card.tsx
function Card({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      className={cn(
        'bg-card text-card-foreground flex flex-col gap-6 rounded-xl border py-6 shadow-sm',
        className,
      )}
      {...props}
    />
  )
}
```

**After:**
```tsx
// card.tsx
function Card({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      className={cn(
        'bg-[var(--card-bg)] text-[var(--semantic-text-primary)] flex flex-col gap-[var(--card-gap)] rounded-[var(--card-radius)] border border-[var(--card-border)] py-[var(--card-padding)] shadow-[var(--card-shadow)]',
        className,
      )}
      {...props}
    />
  )
}
```

### 3. Input Component

**Before:**
```tsx
// input.tsx
function Input({ className, type, ...props }: React.ComponentProps<'input'>) {
  return (
    <input
      className={cn(
        'file:text-foreground placeholder:text-muted-foreground border-input flex h-9 w-full min-w-0 rounded-md border bg-transparent px-3 py-1 text-base shadow-xs',
        'focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]',
        className,
      )}
      {...props}
    />
  )
}
```

**After:**
```tsx
// input.tsx
function Input({ className, type, ...props }: React.ComponentProps<'input'>) {
  return (
    <input
      className={cn(
        'file:text-[var(--semantic-text-primary)] placeholder:text-[var(--input-placeholder-color)] border-[var(--input-border)] flex h-[var(--input-height)] w-full min-w-0 rounded-[var(--input-radius)] border bg-[var(--input-bg)] dark:bg-[var(--input-bg-dark)] px-[var(--input-padding-x)] py-[var(--input-padding-y)] text-[length:var(--input-font-size)] shadow-[var(--semantic-shadow-sm)]',
        'focus-visible:border-[var(--focus-ring-color)] focus-visible:ring-[length:var(--focus-ring-width)] focus-visible:ring-[color:var(--focus-ring-color)] focus-visible:ring-opacity-[var(--focus-ring-opacity)]',
        className,
      )}
      {...props}
    />
  )
}
```

### 4. Task Card Component

**Before:**
```tsx
// task-card.tsx
<Card className={`cursor-pointer transition-colors hover:bg-muted/50`}>
  <CardContent className="p-6">
    <div className="space-y-4">
      <div className="flex items-start justify-between space-x-4">
        <h3 className="text-lg font-semibold leading-none tracking-tight">
          {`Task ${task.taskId.slice(0, 8)}`}
        </h3>
        <Badge variant={task.status === 'error' ? 'destructive' : 'default'}>
          {statusConfig.tooltip}
        </Badge>
      </div>
      <div className="grid grid-cols-2 gap-4 text-sm">
        <div className="flex items-center space-x-2 text-muted-foreground">
          <Calendar className="h-4 w-4" />
          <span>Created: {createdDate.toLocaleDateString()}</span>
        </div>
      </div>
    </div>
  </CardContent>
</Card>
```

**After:**
```tsx
// task-card.tsx
<Card className={`cursor-pointer transition-colors duration-[var(--core-duration-fast)] hover:bg-[var(--semantic-accent)]`}>
  <CardContent className="p-[var(--card-padding)]">
    <div className="space-y-[var(--semantic-spacing-base)]">
      <div className="flex items-start justify-between gap-[var(--semantic-spacing-base)]">
        <h3 className="text-[length:var(--semantic-text-lg)] font-[var(--core-font-weight-semibold)] leading-[var(--core-line-height-none)] tracking-tight">
          {`Task ${task.taskId.slice(0, 8)}`}
        </h3>
        <Badge variant={task.status === 'error' ? 'destructive' : 'default'}>
          {statusConfig.tooltip}
        </Badge>
      </div>
      <div className="grid grid-cols-2 gap-[var(--semantic-spacing-base)] text-[length:var(--semantic-text-sm)]">
        <div className="flex items-center gap-[var(--semantic-spacing-sm)] text-[var(--semantic-text-secondary)]">
          <Calendar className="size-[var(--semantic-spacing-base)]" />
          <span>Created: {createdDate.toLocaleDateString()}</span>
        </div>
      </div>
    </div>
  </CardContent>
</Card>
```

## Utility Class Migrations

### Spacing Utilities

**Before:**
```tsx
// Various spacing patterns
<div className="p-6 px-4 py-2 m-4 gap-2 space-x-4">
<div className="p-[3px]"> // Custom value
```

**After:**
```tsx
// Consistent token-based spacing
<div className="p-[var(--semantic-spacing-lg)] px-[var(--semantic-spacing-base)] py-[var(--semantic-spacing-sm)] m-[var(--semantic-spacing-base)] gap-[var(--semantic-spacing-sm)] space-x-[var(--semantic-spacing-base)]">
<div className="p-[var(--semantic-spacing-xs)]"> // Replaced with closest token
```

### Typography Patterns

**Before:**
```tsx
// Common text patterns
<h2 className="text-lg font-semibold">Title</h2>
<p className="text-sm text-muted-foreground">Description</p>
<span className="text-xs">Caption</span>
```

**After:**
```tsx
// Token-based typography
<h2 className="text-[length:var(--semantic-text-lg)] font-[var(--core-font-weight-semibold)]">Title</h2>
<p className="text-[length:var(--semantic-text-sm)] text-[var(--semantic-text-secondary)]">Description</p>
<span className="text-[length:var(--semantic-text-xs)]">Caption</span>

// Or with composite classes
<h2 className="text-heading-lg">Title</h2>
<p className="text-body text-secondary">Description</p>
<span className="text-caption">Caption</span>
```

### Focus States

**Before:**
```tsx
// Inconsistent focus states
className="focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]"
className="focus-visible:ring-2 focus-visible:ring-primary"
className="focus:outline-none focus:ring-2 focus:ring-offset-2"
```

**After:**
```tsx
// Unified focus state
className="focus-visible:border-[var(--focus-ring-color)] focus-visible:ring-[length:var(--focus-ring-width)] focus-visible:ring-[color:var(--focus-ring-color)] focus-visible:ring-opacity-[var(--focus-ring-opacity)]"

// Or with utility class
className="focus-ring"
```

## Creating New Components with Tokens

### Example: Notification Component

```tsx
// Define component tokens
/* In 03-components.css */
:root {
  --notification-bg: var(--semantic-surface-card);
  --notification-border: var(--semantic-border-primary);
  --notification-radius: var(--semantic-radius-element);
  --notification-padding: var(--semantic-spacing-base);
  --notification-gap: var(--semantic-spacing-sm);
  --notification-icon-size: var(--semantic-spacing-lg);
  
  --notification-success-bg: oklch(from var(--core-emerald-500) l c h / 0.1);
  --notification-success-border: var(--core-emerald-500);
  --notification-success-text: var(--core-emerald-600);
  
  --notification-error-bg: oklch(from var(--semantic-destructive) l c h / 0.1);
  --notification-error-border: var(--semantic-destructive);
  --notification-error-text: var(--semantic-destructive);
}

// Component implementation
function Notification({ variant = 'default', icon, title, message }) {
  const variantClasses = {
    default: 'bg-[var(--notification-bg)] border-[var(--notification-border)]',
    success: 'bg-[var(--notification-success-bg)] border-[var(--notification-success-border)] text-[var(--notification-success-text)]',
    error: 'bg-[var(--notification-error-bg)] border-[var(--notification-error-border)] text-[var(--notification-error-text)]',
  }
  
  return (
    <div className={cn(
      'flex items-start gap-[var(--notification-gap)] p-[var(--notification-padding)] rounded-[var(--notification-radius)] border',
      variantClasses[variant]
    )}>
      {icon && <div className="size-[var(--notification-icon-size)]">{icon}</div>}
      <div>
        <h4 className="text-[length:var(--semantic-text-sm)] font-[var(--core-font-weight-medium)]">{title}</h4>
        <p className="text-[length:var(--semantic-text-sm)] text-[var(--semantic-text-secondary)]">{message}</p>
      </div>
    </div>
  )
}
```

## TypeScript Support

### Token Type Definitions

```typescript
// src/styles/tokens.ts
export const tokens = {
  // Core tokens
  core: {
    colors: {
      white: 'var(--core-white)',
      black: 'var(--core-black)',
      gray: {
        50: 'var(--core-gray-50)',
        100: 'var(--core-gray-100)',
        // ... etc
      },
    },
    spacing: {
      unit: 'var(--core-space-unit)',
      0: 'var(--core-space-0)',
      1: 'var(--core-space-1)',
      // ... etc
    },
  },
  
  // Semantic tokens
  semantic: {
    colors: {
      bg: {
        primary: 'var(--semantic-bg-primary)',
        secondary: 'var(--semantic-bg-secondary)',
        tertiary: 'var(--semantic-bg-tertiary)',
      },
      text: {
        primary: 'var(--semantic-text-primary)',
        secondary: 'var(--semantic-text-secondary)',
        tertiary: 'var(--semantic-text-tertiary)',
      },
    },
    spacing: {
      xs: 'var(--semantic-spacing-xs)',
      sm: 'var(--semantic-spacing-sm)',
      md: 'var(--semantic-spacing-md)',
      // ... etc
    },
  },
  
  // Component tokens
  components: {
    button: {
      height: {
        sm: 'var(--button-height-sm)',
        default: 'var(--button-height-default)',
        lg: 'var(--button-height-lg)',
      },
      // ... etc
    },
  },
} as const

// Type helpers
export type Token = typeof tokens
export type CoreToken = keyof typeof tokens.core
export type SemanticToken = keyof typeof tokens.semantic
export type ComponentToken = keyof typeof tokens.components
```

### Using Tokens in Code

```typescript
// Using tokens with type safety
import { tokens } from '@/styles/tokens'

const styles = {
  padding: tokens.semantic.spacing.md,
  backgroundColor: tokens.semantic.colors.bg.primary,
  borderRadius: tokens.semantic.radius.element,
}

// Custom hook for token values
function useToken(path: string): string {
  return `var(--${path.replace(/\./g, '-')})`
}

// Usage
const bgColor = useToken('semantic.bg.primary') // Returns: var(--semantic-bg-primary)
```

## Migration Checklist for Each Component

1. **Identify all hardcoded values**
   - Colors
   - Spacing (padding, margin, gap)
   - Typography (size, weight, line-height)
   - Border radius
   - Shadows
   - Animation durations

2. **Map to appropriate tokens**
   - Use semantic tokens primarily
   - Use component tokens for overrides
   - Never use core tokens directly

3. **Update class names**
   - Replace Tailwind utilities with token references
   - Use CSS custom property syntax
   - Maintain responsive modifiers

4. **Test thoroughly**
   - Verify in light mode
   - Verify in dark mode
   - Check all interactive states
   - Validate responsive behavior

5. **Document changes**
   - Note any new component tokens created
   - Document any special cases
   - Update component documentation
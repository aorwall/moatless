# Styling with Tailwind CSS v4

## Table of Contents
- [Overview](#overview)
- [Tailwind v4 Setup](#tailwind-v4-setup)
- [Styling Patterns](#styling-patterns)
- [shadcn/ui Components](#shadcnui-components)
- [Responsive Design](#responsive-design)
- [Theme Customization](#theme-customization)
- [Common Patterns](#common-patterns)
- [Anti-Patterns](#anti-patterns)

## Overview

**Tailwind CSS v4** is used for ALL styling in this project.

### Key Principles
- Use Tailwind utility classes directly
- Follow mobile-first responsive design
- Leverage shadcn/ui for consistent component styling
- Avoid custom CSS when possible
- Co-locate styles with components

### Technology Stack
- `tailwindcss` v4 - Utility-first CSS
- `shadcn/ui` - Pre-built accessible components
- CSS-in-JS avoided (use Tailwind)

## Tailwind v4 Setup

### Configuration
Located in `tailwind.config.ts`:

```ts
import type { Config } from 'tailwindcss'

const config: Config = {
  content: [
    './src/**/*.{ts,tsx}'
  ],
  theme: {
    extend: {
      colors: {
        border: 'hsl(var(--border))',
        background: 'hsl(var(--background))',
        foreground: 'hsl(var(--foreground))'
      }
    }
  },
  plugins: []
}

export default config
```

### Global Styles
Located in `src/lib/styles/globals.css`:

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  :root {
    --background: 0 0% 100%;
    --foreground: 222.2 84% 4.9%;
    --border: 214.3 31.8% 91.4%;
  }

  .dark {
    --background: 222.2 84% 4.9%;
    --foreground: 210 40% 98%;
    --border: 217.2 32.6% 17.5%;
  }
}
```

## Styling Patterns

### Basic Component Styling
```tsx
export function Button({ children, variant = 'primary' }: ButtonProps) {
  const baseStyles = 'px-4 py-2 rounded-md font-medium transition-colors'
  const variantStyles = {
    primary: 'bg-blue-600 text-white hover:bg-blue-700',
    secondary: 'bg-gray-200 text-gray-900 hover:bg-gray-300',
    danger: 'bg-red-600 text-white hover:bg-red-700'
  }

  return (
    <button className={`${baseStyles} ${variantStyles[variant]}`}>
      {children}
    </button>
  )
}
```

### Using clsx for Conditional Styles
```tsx
import { clsx } from 'clsx'

interface TaskCardProps {
  title: string
  status: 'pending' | 'running' | 'completed'
  isSelected?: boolean
}

export function TaskCard({ title, status, isSelected }: TaskCardProps) {
  return (
    <div className={clsx(
      'border rounded-lg p-4 transition-colors',
      status === 'running' && 'border-blue-500 bg-blue-50',
      status === 'completed' && 'border-green-500 bg-green-50',
      status === 'pending' && 'border-gray-300',
      isSelected && 'ring-2 ring-blue-500'
    )}>
      <h3 className="font-semibold">{title}</h3>
      <span className={clsx(
        'text-sm',
        status === 'running' && 'text-blue-700',
        status === 'completed' && 'text-green-700',
        status === 'pending' && 'text-gray-600'
      )}>
        {status}
      </span>
    </div>
  )
}
```

### Layout Patterns
```tsx
// Container
<div className="container mx-auto px-4">
  Content
</div>

// Flex layout
<div className="flex items-center justify-between gap-4">
  <div>Left</div>
  <div>Right</div>
</div>

// Grid layout
<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
  {items.map(item => <Card key={item.id} {...item} />)}
</div>

// Centered content
<div className="flex items-center justify-center min-h-screen">
  <div className="max-w-md w-full">Content</div>
</div>
```

## shadcn/ui Components

### Installing Components
```bash
cd apps/frontend
bunx shadcn@latest add button
bunx shadcn@latest add dialog
bunx shadcn@latest add dropdown-menu
```

### Using shadcn/ui Components
```tsx
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger
} from '@/lib/components/ui/dialog'

export function TaskActions({ task }: { task: Task }) {
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button variant="outline">Edit Task</Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit Task</DialogTitle>
        </DialogHeader>
        <TaskForm task={task} />
      </DialogContent>
    </Dialog>
  )
}
```

### Customizing shadcn/ui Components
```tsx
import { Button } from '@/lib/components/ui/button'

// Extend with custom variants
export function IconButton({ icon, ...props }: ButtonProps) {
  return (
    <Button
      {...props}
      className={clsx(
        props.className,
        'w-10 h-10 p-0'  // Custom size for icon buttons
      )}
    >
      {icon}
    </Button>
  )
}
```

## Responsive Design

### Mobile-First Approach
Always start with mobile styles, then add larger breakpoints:

```tsx
<div className="
  w-full            /* Mobile: full width */
  md:w-1/2          /* Tablet: half width */
  lg:w-1/3          /* Desktop: third width */
  xl:w-1/4          /* Large: quarter width */
">
  Content
</div>
```

### Breakpoints
```
sm: 640px    - Small devices
md: 768px    - Medium devices
lg: 1024px   - Large devices
xl: 1280px   - Extra large devices
2xl: 1536px  - 2X large devices
```

### Common Responsive Patterns
```tsx
// Responsive grid
<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
  {items.map(...)}
</div>

// Responsive padding
<div className="p-4 md:p-6 lg:p-8">
  Content
</div>

// Hide on mobile
<div className="hidden md:block">
  Desktop only content
</div>

// Show on mobile only
<div className="block md:hidden">
  Mobile only content
</div>

// Responsive flex direction
<div className="flex flex-col md:flex-row gap-4">
  <div>First</div>
  <div>Second</div>
</div>
```

## Theme Customization

### Dark Mode
```tsx
// ThemeProvider component
import { createContext, useContext, useEffect, useState } from 'react'

type Theme = 'light' | 'dark' | 'system'

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setTheme] = useState<Theme>('system')

  useEffect(() => {
    const root = window.document.documentElement
    root.classList.remove('light', 'dark')

    if (theme === 'system') {
      const systemTheme = window.matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light'
      root.classList.add(systemTheme)
    } else {
      root.classList.add(theme)
    }
  }, [theme])

  return (
    <ThemeContext.Provider value={{ theme, setTheme }}>
      {children}
    </ThemeContext.Provider>
  )
}

// Usage in components
function Header() {
  return (
    <header className="bg-white dark:bg-gray-900 border-b dark:border-gray-800">
      <h1 className="text-gray-900 dark:text-gray-100">Title</h1>
    </header>
  )
}
```

### Custom Colors
Define in `tailwind.config.ts`:

```ts
theme: {
  extend: {
    colors: {
      primary: {
        50: '#f0f9ff',
        100: '#e0f2fe',
        // ... other shades
        900: '#0c4a6e'
      }
    }
  }
}

// Usage
<div className="bg-primary-500 text-primary-50">
  Content
</div>
```

## Common Patterns

### Form Styling
```tsx
export function TaskForm() {
  return (
    <form className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">
          Task Title
        </label>
        <input
          type="text"
          className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">
          Description
        </label>
        <textarea
          className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
          rows={4}
        />
      </div>

      <button
        type="submit"
        className="w-full px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 transition-colors"
      >
        Submit
      </button>
    </form>
  )
}
```

### Loading States
```tsx
export function LoadingSpinner({ size = 'md' }: { size?: 'sm' | 'md' | 'lg' }) {
  const sizeClasses = {
    sm: 'w-4 h-4 border-2',
    md: 'w-8 h-8 border-2',
    lg: 'w-12 h-12 border-4'
  }

  return (
    <div className={clsx(
      'animate-spin rounded-full border-gray-300 border-t-blue-600',
      sizeClasses[size]
    )} />
  )
}

export function LoadingSkeleton() {
  return (
    <div className="animate-pulse">
      <div className="h-4 bg-gray-200 rounded w-3/4 mb-2" />
      <div className="h-4 bg-gray-200 rounded w-1/2" />
    </div>
  )
}
```

### Card Styling
```tsx
export function Card({ children, className }: CardProps) {
  return (
    <div className={clsx(
      'bg-white rounded-lg border border-gray-200 shadow-sm hover:shadow-md transition-shadow',
      className
    )}>
      {children}
    </div>
  )
}

export function CardHeader({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-6 py-4 border-b border-gray-200">
      {children}
    </div>
  )
}

export function CardBody({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-6 py-4">
      {children}
    </div>
  )
}
```

## Anti-Patterns

### ❌ Writing Custom CSS
```tsx
// WRONG
<div style={{ padding: '16px', marginBottom: '8px' }}>
  Content
</div>

// CORRECT
<div className="p-4 mb-2">
  Content
</div>
```

### ❌ Inline Styles for Complex Logic
```tsx
// WRONG
<div style={{
  backgroundColor: status === 'active' ? '#00ff00' : '#ff0000',
  padding: '8px',
  borderRadius: '4px'
}}>
  Content
</div>

// CORRECT
<div className={clsx(
  'p-2 rounded',
  status === 'active' ? 'bg-green-500' : 'bg-red-500'
)}>
  Content
</div>
```

### ❌ Not Using Mobile-First
```tsx
// WRONG - Desktop first
<div className="w-1/3 sm:w-full">
  Content
</div>

// CORRECT - Mobile first
<div className="w-full sm:w-1/3">
  Content
</div>
```

### ❌ Hardcoding Colors
```tsx
// WRONG
<div className="bg-[#3b82f6]">
  Content
</div>

// CORRECT - Use theme colors
<div className="bg-blue-600">
  Content
</div>
```

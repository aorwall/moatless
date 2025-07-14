# Styling Guide

## 🎯 Our Approach: shadcn/ui First

We use shadcn/ui as our primary component library with minimal customization. This pragmatic approach:
- Preserves the polished shadcn/ui aesthetic
- Maintains excellent developer experience  
- Fixes specific styling issues (colors, borders, themes)
- Allows gradual enhancement without disruption

## 💡 Key Principles

1. **Don't Fight the Framework** - shadcn/ui works well, use it as designed
2. **Fix Real Problems** - Focus on actual issues (colors, contrast, themes)
3. **Progressive Enhancement** - Add complexity only when needed
4. **Developer Experience** - Keep it simple and maintainable

## 🎨 Color System

### Semantic Color Tokens

We use semantic color tokens that automatically adapt between light and dark modes. This approach:
- **Eliminates hardcoded colors** - No more `text-gray-500` or `bg-gray-100`
- **Automatic theme switching** - No need for `dark:` classes in components
- **Easy color updates** - Change colors in one place (globals.css)
- **Consistent theming** - All components respect the current theme

### Core Semantic Tokens

These CSS variables automatically switch values based on theme:

```css
/* Light Mode (default) */
:root {
  /* Backgrounds */
  --background: rgb(252 251 248);        /* Main page background */
  --foreground: oklch(0.145 0 0);        /* Main text color */
  
  /* Surface colors */
  --card: oklch(0.98 0 0);               /* Card/panel background */
  --card-foreground: oklch(0.145 0 0);   /* Text on cards */
  
  /* Interactive elements */
  --primary: oklch(0.205 0 0);           /* Primary actions */
  --primary-foreground: oklch(0.985 0 0); /* Text on primary */
  --secondary: oklch(0.956 0.04 85.87);  /* Secondary actions */
  --secondary-foreground: oklch(0.205 0 0);
  
  /* UI states */
  --muted: oklch(0.96 0.004 106.66);     /* Muted backgrounds */
  --muted-foreground: oklch(0.556 0 0);  /* Muted text */
  --accent: oklch(0.97 0 0);             /* Hover states */
  --accent-foreground: oklch(0.205 0 0);
  
  /* Semantic states */
  --destructive: oklch(0.577 0.245 27.325);
  --destructive-foreground: oklch(0.985 0 0);
  
  /* Status states - NEW! */
  --status-info: oklch(0.598 0.155 231.528);      /* Info/Open states */
  --status-info-foreground: oklch(0.985 0 0);
  --status-success: oklch(0.622 0.178 149.882);   /* Success/Complete */
  --status-success-foreground: oklch(0.985 0 0);
  --status-warning: oklch(0.768 0.149 84.929);    /* Warning/Running */
  --status-warning-foreground: oklch(0.205 0 0);
  --status-error: oklch(0.577 0.245 27.325);      /* Error states */
  --status-error-foreground: oklch(0.985 0 0);
  --status-pending: oklch(0.708 0 0);             /* Pending/Muted */
  --status-pending-foreground: oklch(0.985 0 0);
  
  /* Borders and inputs */
  --border: oklch(0.9 0.004 106.66);     /* Default borders */
  --input: oklch(0.922 0 0);             /* Input backgrounds */
  --ring: oklch(0.708 0 0);              /* Focus rings */
}

/* Dark Mode - Same variables, different values */
.dark {
  --background: oklch(0.216 0 0);
  --foreground: oklch(0.985 0 0);
  --card: oklch(0.05 0 0);
  --card-foreground: oklch(0.98 0.04 45);
  /* ... all other variables automatically switch */
}
```

### Usage - No Conditional Classes Needed

```tsx
// ✅ CORRECT - Automatic theme switching
<div className="bg-background text-foreground">
  <Card className="border-border">
    <p className="text-muted-foreground">
      This text automatically adapts to light/dark mode
    </p>
  </Card>
</div>

// ❌ AVOID - Hardcoded colors break theming
<div className="bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100">
  <Card className="border-gray-200 dark:border-gray-800">
    <p className="text-gray-500 dark:text-gray-400">
      This requires manual dark mode handling
    </p>
  </Card>
</div>

// ❌ NEVER - Hardcoded color values
<div className="bg-gray-50 text-gray-900">
  <p className="text-gray-500">This won't adapt to dark mode at all</p>
</div>
```

### Status Colors - Semantic State Indicators

Use semantic status colors for consistent state representation across the application:

```tsx
// ✅ CORRECT - Using semantic status colors
function TaskStatus({ status }: { status: string }) {
  const statusConfig = {
    open: {
      icon: <CircleIcon className="h-4 w-4" />,
      className: 'text-status-info bg-status-info/10 border-status-info/20',
    },
    running: {
      icon: <Loader2 className="h-4 w-4 animate-spin" />,
      className: 'text-status-warning bg-status-warning/10 border-status-warning/20',
    },
    error: {
      icon: <AlertCircle className="h-4 w-4" />,
      className: 'text-status-error bg-status-error/10 border-status-error/20',
    },
    complete: {
      icon: <CheckCircle className="h-4 w-4" />,
      className: 'text-status-success bg-status-success/10 border-status-success/20',
    },
  }
  
  const config = statusConfig[status] || statusConfig.open
  
  return (
    <Badge variant="outline" className={config.className}>
      {config.icon}
      <span className="ml-1">{status}</span>
    </Badge>
  )
}

// ❌ AVOID - Hardcoded status colors
function BadTaskStatus({ status }: { status: string }) {
  const colors = {
    open: 'text-blue-600 dark:text-blue-400 bg-blue-50 dark:bg-blue-900/20',
    running: 'text-yellow-600 dark:text-yellow-400 bg-yellow-50 dark:bg-yellow-900/20',
    error: 'text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20',
    complete: 'text-green-600 dark:text-green-400 bg-green-50 dark:bg-green-900/20',
  }
  
  return <Badge className={colors[status]}>{status}</Badge>
}
```

### Semantic Token Reference

| Token | Purpose | Example Usage |
|-------|---------|---------------|
| `background` | Page background | `bg-background` |
| `foreground` | Default text | `text-foreground` |
| `card` | Card backgrounds | `bg-card` |
| `card-foreground` | Text on cards | `text-card-foreground` |
| `primary` | Primary buttons/links | `bg-primary text-primary-foreground` |
| `secondary` | Secondary actions | `bg-secondary text-secondary-foreground` |
| `muted` | Subtle backgrounds | `bg-muted` |
| `muted-foreground` | Subtle text | `text-muted-foreground` |
| `accent` | Hover states | `hover:bg-accent hover:text-accent-foreground` |
| `destructive` | Errors/critical actions | `bg-destructive text-destructive-foreground` |
| `border` | All borders | `border-border` |
| `input` | Form inputs | `bg-input` |
| `ring` | Focus rings | `ring-ring` |
| **Status Colors** | **Semantic States** | **Example Usage** |
| `status-info` | Info messages, open states | `text-status-info bg-status-info/10` |
| `status-success` | Success, complete, merged | `text-status-success bg-status-success/10` |
| `status-warning` | Warnings, running states | `text-status-warning bg-status-warning/10` |
| `status-error` | Errors, failed states | `text-status-error bg-status-error/10` |
| `status-pending` | Pending, closed, muted | `text-status-pending bg-status-pending/10` |

### Customizing Colors

To change the color scheme, update the values in `globals.css`:

```css
/* Example: Change to a blue primary color */
:root {
  --primary: oklch(0.6 0.2 240);        /* Blue instead of black */
  --primary-foreground: oklch(1 0 0);   /* White text on blue */
}

.dark {
  --primary: oklch(0.7 0.15 240);      /* Lighter blue for dark mode */
  --primary-foreground: oklch(0.1 0 0); /* Dark text on light blue */
}
```

All components using `bg-primary` or `text-primary` will automatically update.

## 🛠️ Component Guidelines

### Component Hierarchy

1. **shadcn/ui Components** (Use First) - Button, Card, Input, Badge, etc.
2. **Feature Components** (Compose Second) - Combine shadcn/ui with business logic
3. **Custom Components** (Create Last) - Only for unique brand needs

### Usage Examples

#### ✅ Level 1: Use shadcn/ui Directly

```tsx
import { Button } from '@/lib/features/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/features/ui/card'

export function TaskCard({ task, onComplete }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{task.title}</CardTitle>
      </CardHeader>
      <CardContent>
        <p className="text-muted-foreground">{task.description}</p>
        <Button 
          onClick={() => onComplete(task.id)}
          variant="secondary"
          size="sm"
          className="mt-4"
        >
          Mark Complete
        </Button>
      </CardContent>
    </Card>
  )
}
```

#### ✅ Level 2: Compose for Features

```tsx
import { Badge } from '@/lib/features/ui/badge'
import { Button } from '@/lib/features/ui/button'
import { Card } from '@/lib/features/ui/card'

export function MetricCard({ title, value, change, trend, onViewDetails }) {
  return (
    <Card className="relative overflow-hidden">
      <div className="p-6">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium text-muted-foreground">{title}</h3>
          <Badge variant={trend === 'up' ? 'default' : 'destructive'}>
            {change}%
          </Badge>
        </div>
        <div className="mt-2">
          <span className="text-2xl font-bold">{value}</span>
        </div>
        <Button variant="ghost" size="sm" onClick={onViewDetails} className="mt-4">
          View Details →
        </Button>
      </div>
    </Card>
  )
}
```

#### ⚠️ Level 3: Custom Components (Rare)

```tsx
// Only when shadcn/ui truly doesn't meet the need
export function BrandHero({ title, subtitle, cta }) {
  return (
    <section className="relative overflow-hidden bg-[rgb(252,251,248)] dark:bg-gray-950">
      <div className="absolute inset-0 bg-gradient-to-br from-primary/5 to-transparent" />
      <div className="relative mx-auto max-w-7xl px-6 py-24">
        <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        <p className="mt-6 text-lg text-muted-foreground">{subtitle}</p>
        <div className="mt-10">{cta}</div>
      </div>
    </section>
  )
}
```

## 📏 Best Practices

### Do's ✅

1. **Use theme-aware colors**
   ```tsx
   className="text-muted-foreground bg-muted border-border"
   ```

2. **Leverage Tailwind utilities**
   ```tsx
   className="mt-4 space-y-2 rounded-lg hover:shadow-lg"
   ```

3. **Compose with shadcn/ui**
   ```tsx
   <Card className="hover:shadow-lg transition-shadow">
     <CardContent className="p-0">
       {/* Custom content */}
     </CardContent>
   </Card>
   ```

4. **Use built-in variants**
   ```tsx
   <Button variant="secondary" size="lg" />
   <Badge variant="outline" />
   <Alert variant="destructive" />
   ```

### Don'ts ❌

1. **Don't use hardcoded colors**
   ```tsx
   // ❌ BAD - Breaks theme switching
   <div className="text-gray-500 border-gray-300" />
   <p className="text-slate-600 bg-zinc-100" />
   
   // ✅ GOOD - Use semantic tokens
   <div className="text-muted-foreground border-border" />
   <p className="text-foreground bg-muted" />
   ```

2. **Don't use dark: prefixes**
   ```tsx
   // ❌ BAD - Manual theme handling
   <div className="bg-white dark:bg-gray-900 text-black dark:text-white" />
   
   // ✅ GOOD - Automatic theme switching
   <div className="bg-background text-foreground" />
   ```

3. **Don't override component internals**
   ```tsx
   // ❌ BAD
   <Button className="!bg-red-500 !text-white" />
   
   // ✅ GOOD - Use variants
   <Button variant="destructive" />
   ```

4. **Don't recreate existing components**
   ```tsx
   // ❌ BAD: Creating MyButton when Button exists
   export function MyButton({ children }) {
     return <button className="px-4 py-2 ...">{children}</button>
   }
   ```

## 🔧 Common Patterns

### Loading States
```tsx
import { Skeleton } from '@/lib/features/ui/skeleton'

export function TaskCardSkeleton() {
  return (
    <Card>
      <CardHeader>
        <Skeleton className="h-6 w-48" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-full" />
        <Skeleton className="h-4 w-3/4 mt-2" />
      </CardContent>
    </Card>
  )
}
```

### Empty States
```tsx
export function EmptyTaskList() {
  return (
    <Card className="flex flex-col items-center justify-center p-12 text-center">
      <p className="text-muted-foreground">No tasks yet</p>
      <Button className="mt-4" variant="secondary">
        Create First Task
      </Button>
    </Card>
  )
}
```

### Error States
```tsx
import { Alert, AlertDescription } from '@/lib/features/ui/alert'

export function ErrorMessage({ error }) {
  return (
    <Alert variant="destructive">
      <AlertDescription>
        {error.message || 'Something went wrong'}
      </AlertDescription>
    </Alert>
  )
}
```

## 🚀 Implementation Checklist

### Before Creating a New Component

1. ✓ Does shadcn/ui have something similar?
2. ✓ Can I compose existing components?
3. ✓ Is this a one-off or reusable pattern?
4. ✓ Have I checked with the team?
5. ✓ Is there a clear reason to diverge?

### Before Submitting PR

1. ✓ Used shadcn/ui components where possible
2. ✓ No hardcoded colors (use `text-muted-foreground`, etc.)
3. ✓ Theme switching works (test light/dark modes)
4. ✓ Responsive design considered
5. ✓ Accessibility checked
6. ✓ Component documented if custom

## 🔍 Debugging Color Issues

### Quick Checks

```bash
# Find hardcoded colors (gray, slate, zinc, stone, etc.)
grep -r "text-gray-\|text-slate-\|text-zinc-\|text-stone-" --include="*.tsx" --include="*.jsx"
grep -r "bg-gray-\|bg-slate-\|bg-zinc-\|bg-stone-" --include="*.tsx" --include="*.jsx"
grep -r "border-gray-\|border-slate-\|border-zinc-" --include="*.tsx" --include="*.jsx"

# Find dark: classes that should use semantic tokens
grep -r "dark:" --include="*.tsx" --include="*.jsx" | grep -v "dark:.*-foreground\|dark:.*-background"

# Find white/black hardcoded values
grep -r "bg-white\|bg-black\|text-white\|text-black" --include="*.tsx" --include="*.jsx"
```

### Common Problems

1. **Colors look different than expected**
   - Check if OKLCH is supported in your browser
   - Use RGB fallbacks for critical colors
   - Verify no CSS transforms are applied

2. **Theme switching issues**
   - Ensure `.dark` class is on `<html>` element
   - Check specificity of color classes
   - Verify no hardcoded colors

3. **Contrast problems**
   - Use DevTools to check actual color values
   - Test with accessibility tools
   - Adjust lightness values as needed

## 📁 File Organization

```
src/
├── lib/
│   ├── components/
│   │   └── ui/           # shadcn/ui components (don't modify)
│   │       ├── button.tsx
│   │       ├── card.tsx
│   │       └── ...
│   └── styles/
│       └── globals.css   # Semantic color tokens defined here
├── features/
│   └── [feature]/
│       └── components/   # Feature-specific components
│           ├── task-list.tsx
│           └── ...
└── components/          # Shared custom components (rare)
    └── brand/
        └── hero.tsx
```

## 🎨 When to Use What

### Use shadcn/ui Variables:
- All shadcn/ui components (90% of cases)
- Most UI elements
- Standard layouts
- Common patterns

### Use Custom Styling:
- Brand-specific components
- Marketing pages
- Unique interactions
- Gradual enhancements

### Create New Styles:
- Only when shadcn/ui doesn't provide what you need
- With clear documentation
- Following existing patterns
- With team consensus

## 🏁 Summary

Our styling strategy prioritizes:
- **Consistency** through shadcn/ui
- **Flexibility** through Tailwind CSS  
- **Maintainability** through minimal abstractions
- **Quality** through fixing real issues

The design tokens remain available for future needs but are not required for most development. This approach gives us the best of both worlds: a solid foundation with room to grow.

---

For questions about styling, always start with "Can I use shadcn/ui for this?" The answer is usually yes.
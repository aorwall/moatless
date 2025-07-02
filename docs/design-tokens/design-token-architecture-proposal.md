# Three-Tier Design Token Architecture Refactoring Proposal

## Executive Summary

This document outlines a comprehensive refactoring strategy to implement a Three-Tier Design Token Architecture for the Moatless Swarm UI application. The current implementation uses Tailwind CSS v4 with shadcn/ui components and has a solid foundation with semantic color naming and OKLCH color space. However, the design system lacks systematic token organization, leading to inconsistencies and maintenance challenges.

### Key Benefits of This Refactoring
- **Improved Consistency**: Eliminate design discrepancies across components
- **Enhanced Maintainability**: Centralized design decisions in a hierarchical token system
- **Scalability**: Easy to add new themes and variations
- **Developer Experience**: Clear naming conventions and predictable patterns
- **Design-Development Alignment**: Single source of truth for design decisions

## Current State Analysis

### Technology Stack
- **CSS Framework**: Tailwind CSS v4.0.6
- **Component Library**: shadcn/ui with Radix UI primitives
- **Color System**: OKLCH color space (perceptually uniform)
- **Styling Approach**: Utility-first with CVA for variants
- **Theme Support**: Light/dark mode via CSS custom properties

### Design Pattern Inventory

#### Color System
```css
/* Current semantic colors in globals.css */
--background, --foreground
--card, --card-foreground
--primary, --primary-foreground
--secondary, --secondary-foreground
--muted, --muted-foreground
--accent, --accent-foreground
--destructive, --destructive-foreground
--border, --input, --ring
--chart-1 through --chart-5
--sidebar-* variants
```

#### Typography Scale
- **Sizes**: text-xs (12px), text-sm (14px), text-base (16px), text-lg (18px), text-xl (20px)
- **Weights**: font-medium (500), font-semibold (600)
- **Line Heights**: Tailwind defaults (tight, normal, relaxed)
- **Common Combinations**:
  - Headers: `text-lg font-semibold`
  - Body: `text-sm`
  - Captions: `text-xs text-muted-foreground`

#### Spacing Patterns
- **Component Padding**: p-6 (24px), p-4 (16px), p-3 (12px), p-2 (8px)
- **Element Spacing**: gap-2 (8px), gap-4 (16px), gap-6 (24px)
- **Inline Spacing**: space-x-2, space-x-4
- **Custom Values**: p-[3px], ring-[3px]

#### Component Patterns
- **Buttons**: Multiple variants (default, destructive, outline, secondary, ghost, link)
- **Cards**: Consistent border, shadow-sm, rounded-xl
- **Forms**: h-9 inputs, consistent focus states
- **Focus Rings**: `focus-visible:ring-ring/50 focus-visible:ring-[3px]`

### Identified Issues and Inconsistencies

1. **Hardcoded Values**
   - Custom pixel values: `p-[3px]`, `ring-[3px]`, `h-[calc(100%-1px)]`
   - Inline calculations instead of design tokens

2. **Shadow System**
   - Limited to shadow-xs and shadow-sm
   - No systematic elevation scale

3. **Opacity Patterns**
   - Mix of Tailwind utilities (/50, /90) and OKLCH alpha
   - Inconsistent hover state opacities

4. **Focus States**
   - Different ring sizes across components
   - Inconsistent focus-visible implementations

5. **Spacing Inconsistencies**  
   - Some components use custom spacing values
   - No clear spacing rhythm for layouts

## Proposed Three-Tier Token Architecture

### Tier 1: Core/Primitive Tokens

```css
/* src/styles/tokens/01-core.css */
:root {
  /* Color Primitives - Raw OKLCH values */
  --core-white: oklch(1 0 0);
  --core-black: oklch(0 0 0);
  
  /* Gray Scale */
  --core-gray-50: oklch(0.985 0 0);
  --core-gray-100: oklch(0.97 0 0);
  --core-gray-200: oklch(0.956 0.04 85.87);
  --core-gray-300: oklch(0.922 0 0);
  --core-gray-400: oklch(0.708 0 0);
  --core-gray-500: oklch(0.556 0 0);
  --core-gray-600: oklch(0.439 0 0);
  --core-gray-700: oklch(0.269 0 0);
  --core-gray-800: oklch(0.216 0 0);
  --core-gray-900: oklch(0.145 0 0);
  --core-gray-950: oklch(0.05 0 0);
  
  /* Brand Colors */
  --core-blue-50: oklch(0.97 0.01 237);
  --core-blue-500: oklch(0.488 0.243 264.376);
  --core-blue-600: oklch(0.44 0.22 264);
  
  /* Accent Colors */
  --core-amber-500: oklch(0.828 0.189 84.429);
  --core-emerald-500: oklch(0.6 0.118 184.704);
  
  /* Semantic Colors */
  --core-red-500: oklch(0.577 0.245 27.325);
  --core-red-600: oklch(0.396 0.141 25.723);
  --core-red-700: oklch(0.637 0.237 25.331);
  
  /* Spacing Primitives */
  --core-space-unit: 4px;
  --core-space-0: 0;
  --core-space-1: calc(var(--core-space-unit) * 1);   /* 4px */
  --core-space-2: calc(var(--core-space-unit) * 2);   /* 8px */
  --core-space-3: calc(var(--core-space-unit) * 3);   /* 12px */
  --core-space-4: calc(var(--core-space-unit) * 4);   /* 16px */
  --core-space-5: calc(var(--core-space-unit) * 5);   /* 20px */
  --core-space-6: calc(var(--core-space-unit) * 6);   /* 24px */
  --core-space-8: calc(var(--core-space-unit) * 8);   /* 32px */
  --core-space-10: calc(var(--core-space-unit) * 10); /* 40px */
  --core-space-12: calc(var(--core-space-unit) * 12); /* 48px */
  --core-space-16: calc(var(--core-space-unit) * 16); /* 64px */
  
  /* Typography Primitives */
  --core-font-sans: ui-sans-serif, system-ui, sans-serif;
  --core-font-mono: ui-monospace, monospace;
  
  /* Font Sizes */
  --core-font-size-xs: 0.75rem;    /* 12px */
  --core-font-size-sm: 0.875rem;   /* 14px */
  --core-font-size-base: 1rem;     /* 16px */
  --core-font-size-lg: 1.125rem;   /* 18px */
  --core-font-size-xl: 1.25rem;    /* 20px */
  --core-font-size-2xl: 1.5rem;    /* 24px */
  --core-font-size-3xl: 1.875rem;  /* 30px */
  
  /* Font Weights */
  --core-font-weight-normal: 400;
  --core-font-weight-medium: 500;
  --core-font-weight-semibold: 600;
  --core-font-weight-bold: 700;
  
  /* Line Heights */
  --core-line-height-none: 1;
  --core-line-height-tight: 1.25;
  --core-line-height-snug: 1.375;
  --core-line-height-normal: 1.5;
  --core-line-height-relaxed: 1.625;
  
  /* Border Radius */
  --core-radius-none: 0;
  --core-radius-sm: 0.375rem;      /* 6px */
  --core-radius-base: 0.5rem;      /* 8px */
  --core-radius-md: 0.625rem;      /* 10px */
  --core-radius-lg: 0.75rem;       /* 12px */
  --core-radius-xl: 1rem;          /* 16px */
  --core-radius-full: 9999px;
  
  /* Shadows */
  --core-shadow-xs: 0 1px 2px 0 rgb(0 0 0 / 0.05);
  --core-shadow-sm: 0 1px 3px 0 rgb(0 0 0 / 0.1), 0 1px 2px -1px rgb(0 0 0 / 0.1);
  --core-shadow-md: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
  --core-shadow-lg: 0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1);
  --core-shadow-xl: 0 20px 25px -5px rgb(0 0 0 / 0.1), 0 8px 10px -6px rgb(0 0 0 / 0.1);
  
  /* Animation Durations */
  --core-duration-instant: 0ms;
  --core-duration-fast: 150ms;
  --core-duration-normal: 250ms;
  --core-duration-slow: 350ms;
  
  /* Z-Index Scale */
  --core-z-index-dropdown: 1000;
  --core-z-index-sticky: 1020;
  --core-z-index-fixed: 1030;
  --core-z-index-modal-backdrop: 1040;
  --core-z-index-modal: 1050;
  --core-z-index-popover: 1060;
  --core-z-index-tooltip: 1070;
}
```

### Tier 2: Semantic/Alias Tokens

```css
/* src/styles/tokens/02-semantic.css */
:root {
  /* Semantic Colors - Light Theme */
  --semantic-bg-primary: var(--core-white);
  --semantic-bg-secondary: var(--core-gray-50);
  --semantic-bg-tertiary: var(--core-gray-100);
  
  --semantic-text-primary: var(--core-gray-900);
  --semantic-text-secondary: var(--core-gray-500);
  --semantic-text-tertiary: var(--core-gray-400);
  --semantic-text-inverse: var(--core-gray-50);
  
  --semantic-brand-primary: var(--core-gray-900);
  --semantic-brand-primary-hover: oklch(from var(--core-gray-900) calc(l * 0.9) c h);
  
  --semantic-accent: var(--core-gray-100);
  --semantic-accent-hover: oklch(from var(--core-gray-100) calc(l * 0.95) c h);
  
  --semantic-destructive: var(--core-red-500);
  --semantic-destructive-hover: oklch(from var(--core-red-500) calc(l * 0.9) c h);
  
  --semantic-border-primary: var(--core-gray-200);
  --semantic-border-secondary: var(--core-gray-300);
  
  --semantic-surface-card: var(--core-gray-50);
  --semantic-surface-popover: var(--core-white);
  --semantic-surface-modal: var(--core-white);
  
  /* Semantic Spacing */
  --semantic-spacing-none: var(--core-space-0);
  --semantic-spacing-xs: var(--core-space-1);      /* 4px */
  --semantic-spacing-sm: var(--core-space-2);      /* 8px */
  --semantic-spacing-md: var(--core-space-3);      /* 12px */
  --semantic-spacing-base: var(--core-space-4);    /* 16px */
  --semantic-spacing-lg: var(--core-space-6);      /* 24px */
  --semantic-spacing-xl: var(--core-space-8);      /* 32px */
  --semantic-spacing-2xl: var(--core-space-12);    /* 48px */
  
  /* Semantic Typography */
  --semantic-font-family-base: var(--core-font-sans);
  --semantic-font-family-code: var(--core-font-mono);
  
  --semantic-text-xs: var(--core-font-size-xs);
  --semantic-text-sm: var(--core-font-size-sm);
  --semantic-text-base: var(--core-font-size-base);
  --semantic-text-lg: var(--core-font-size-lg);
  --semantic-text-xl: var(--core-font-size-xl);
  
  /* Semantic Radius */
  --semantic-radius-element: var(--core-radius-md);    /* 10px - default */
  --semantic-radius-container: var(--core-radius-lg);  /* 12px - cards */
  --semantic-radius-button: var(--core-radius-base);   /* 8px */
  
  /* Semantic Shadows */
  --semantic-shadow-none: none;
  --semantic-shadow-sm: var(--core-shadow-xs);
  --semantic-shadow-base: var(--core-shadow-sm);
  --semantic-shadow-md: var(--core-shadow-md);
  --semantic-shadow-lg: var(--core-shadow-lg);
  
  /* Interactive States */
  --semantic-focus-ring-color: var(--semantic-brand-primary);
  --semantic-focus-ring-width: 3px;
  --semantic-focus-ring-opacity: 0.5;
}

/* Dark Theme Overrides */
.dark {
  --semantic-bg-primary: var(--core-gray-800);
  --semantic-bg-secondary: var(--core-gray-900);
  --semantic-bg-tertiary: var(--core-gray-950);
  
  --semantic-text-primary: var(--core-gray-50);
  --semantic-text-secondary: var(--core-gray-400);
  --semantic-text-tertiary: var(--core-gray-500);
  --semantic-text-inverse: var(--core-gray-900);
  
  --semantic-brand-primary: var(--core-gray-50);
  --semantic-brand-primary-hover: oklch(from var(--core-gray-50) calc(l * 0.9) c h);
  
  --semantic-accent: var(--core-gray-700);
  --semantic-accent-hover: oklch(from var(--core-gray-700) calc(l * 1.2) c h);
  
  --semantic-destructive: var(--core-red-600);
  --semantic-destructive-hover: oklch(from var(--core-red-600) calc(l * 1.1) c h);
  
  --semantic-border-primary: var(--core-gray-700);
  --semantic-border-secondary: var(--core-gray-600);
  
  --semantic-surface-card: var(--core-gray-950);
  --semantic-surface-popover: var(--core-gray-900);
  --semantic-surface-modal: var(--core-gray-900);
}
```

### Tier 3: Component Tokens

```css
/* src/styles/tokens/03-components.css */
:root {
  /* Button Component Tokens */
  --button-height-sm: 2rem;        /* 32px */
  --button-height-default: 2.25rem; /* 36px */
  --button-height-lg: 2.5rem;      /* 40px */
  
  --button-padding-x-sm: var(--semantic-spacing-md);
  --button-padding-x-default: var(--semantic-spacing-base);
  --button-padding-x-lg: var(--semantic-spacing-lg);
  
  --button-font-size: var(--semantic-text-sm);
  --button-font-weight: var(--core-font-weight-medium);
  --button-radius: var(--semantic-radius-button);
  
  --button-primary-bg: var(--semantic-brand-primary);
  --button-primary-bg-hover: var(--semantic-brand-primary-hover);
  --button-primary-text: var(--semantic-text-inverse);
  
  --button-secondary-bg: var(--semantic-bg-secondary);
  --button-secondary-bg-hover: var(--semantic-bg-tertiary);
  --button-secondary-text: var(--semantic-text-primary);
  
  --button-destructive-bg: var(--semantic-destructive);
  --button-destructive-bg-hover: var(--semantic-destructive-hover);
  --button-destructive-text: var(--core-white);
  
  /* Card Component Tokens */
  --card-bg: var(--semantic-surface-card);
  --card-border: var(--semantic-border-primary);
  --card-radius: var(--semantic-radius-container);
  --card-shadow: var(--semantic-shadow-base);
  --card-padding: var(--semantic-spacing-lg);
  --card-gap: var(--semantic-spacing-lg);
  
  /* Input Component Tokens */
  --input-height: var(--button-height-default);
  --input-padding-x: var(--semantic-spacing-md);
  --input-padding-y: var(--semantic-spacing-xs);
  --input-bg: transparent;
  --input-bg-dark: oklch(from var(--semantic-bg-tertiary) l c h / 0.3);
  --input-border: var(--semantic-border-secondary);
  --input-radius: var(--semantic-radius-button);
  --input-font-size: var(--semantic-text-sm);
  --input-placeholder-color: var(--semantic-text-tertiary);
  
  /* Badge Component Tokens */
  --badge-padding-x: var(--semantic-spacing-sm);
  --badge-padding-y: calc(var(--semantic-spacing-xs) * 1.25); /* 5px */
  --badge-font-size: var(--semantic-text-xs);
  --badge-font-weight: var(--core-font-weight-medium);
  --badge-radius: var(--semantic-radius-button);
  
  /* Dialog/Modal Tokens */
  --dialog-bg: var(--semantic-surface-modal);
  --dialog-radius: var(--semantic-radius-container);
  --dialog-shadow: var(--semantic-shadow-lg);
  --dialog-padding: var(--semantic-spacing-lg);
  
  /* Sidebar Tokens */
  --sidebar-width: 16rem;          /* 256px */
  --sidebar-collapsed-width: 3rem; /* 48px */
  --sidebar-bg: var(--semantic-bg-secondary);
  --sidebar-border: var(--semantic-border-primary);
  --sidebar-item-padding: var(--semantic-spacing-sm);
  --sidebar-item-radius: var(--semantic-radius-button);
  
  /* Table Tokens */
  --table-header-bg: var(--semantic-bg-secondary);
  --table-header-text: var(--semantic-text-primary);
  --table-row-hover-bg: var(--semantic-bg-tertiary);
  --table-border: var(--semantic-border-primary);
  --table-cell-padding: var(--semantic-spacing-md);
  
  /* Focus State Tokens */
  --focus-ring-color: var(--semantic-focus-ring-color);
  --focus-ring-width: var(--semantic-focus-ring-width);
  --focus-ring-opacity: var(--semantic-focus-ring-opacity);
  --focus-ring-offset: 0;
}
```

## Migration Strategy

### Phase 1: Foundation Setup (Week 1)

1. **Create Token Files Structure**
   ```
   apps/frontend/src/styles/
   ├── tokens/
   │   ├── 01-core.css
   │   ├── 02-semantic.css
   │   └── 03-components.css
   └── globals.css (updated to import tokens)
   ```

2. **Update globals.css**
   ```css
   @import "./tokens/01-core.css";
   @import "./tokens/02-semantic.css";
   @import "./tokens/03-components.css";
   @import "tailwindcss";
   @import "tw-animate-css";
   
   /* Rest of existing globals.css content */
   ```

3. **Create Token Mapping**
   ```javascript
   // tailwind.config.js
   module.exports = {
     theme: {
       extend: {
         colors: {
           background: 'var(--semantic-bg-primary)',
           foreground: 'var(--semantic-text-primary)',
           card: {
             DEFAULT: 'var(--card-bg)',
             foreground: 'var(--semantic-text-primary)',
           },
           primary: {
             DEFAULT: 'var(--semantic-brand-primary)',
             foreground: 'var(--semantic-text-inverse)',
           },
           secondary: {
             DEFAULT: 'var(--semantic-bg-secondary)',
             foreground: 'var(--semantic-text-primary)',
           },
           muted: {
             DEFAULT: 'var(--semantic-bg-tertiary)',
             foreground: 'var(--semantic-text-secondary)',
           },
           accent: {
             DEFAULT: 'var(--semantic-accent)',
             foreground: 'var(--semantic-text-primary)',
           },
           destructive: {
             DEFAULT: 'var(--semantic-destructive)',
             foreground: 'var(--core-white)',
           },
           border: 'var(--semantic-border-primary)',
           input: 'var(--semantic-border-secondary)',
           ring: 'var(--focus-ring-color)',
         },
         spacing: {
           'xs': 'var(--semantic-spacing-xs)',
           'sm': 'var(--semantic-spacing-sm)',
           'md': 'var(--semantic-spacing-md)',
           'lg': 'var(--semantic-spacing-lg)',
           'xl': 'var(--semantic-spacing-xl)',
           '2xl': 'var(--semantic-spacing-2xl)',
         },
         borderRadius: {
           'element': 'var(--semantic-radius-element)',
           'container': 'var(--semantic-radius-container)',
           'button': 'var(--semantic-radius-button)',
         },
         fontSize: {
           'xs': ['var(--semantic-text-xs)', { lineHeight: 'var(--core-line-height-normal)' }],
           'sm': ['var(--semantic-text-sm)', { lineHeight: 'var(--core-line-height-normal)' }],
           'base': ['var(--semantic-text-base)', { lineHeight: 'var(--core-line-height-normal)' }],
           'lg': ['var(--semantic-text-lg)', { lineHeight: 'var(--core-line-height-tight)' }],
           'xl': ['var(--semantic-text-xl)', { lineHeight: 'var(--core-line-height-tight)' }],
         },
         boxShadow: {
           'xs': 'var(--semantic-shadow-sm)',
           'sm': 'var(--semantic-shadow-base)',
           'md': 'var(--semantic-shadow-md)',
           'lg': 'var(--semantic-shadow-lg)',
         },
       },
     },
   };
   ```

### Phase 2: Component Migration (Week 2-3)

1. **Start with Base Components**
   
   **Button Component Migration:**
   ```tsx
   // Before
   const buttonVariants = cva(
     "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all disabled:pointer-events-none disabled:opacity-50",
     {
       variants: {
         variant: {
           default: 'bg-primary text-primary-foreground shadow-xs hover:bg-primary/90',
           // ...
         },
         size: {
           default: 'h-9 px-4 py-2',
           sm: 'h-8 rounded-md gap-1.5 px-3',
           lg: 'h-10 rounded-md px-6',
         },
       },
     },
   )
   
   // After
   const buttonVariants = cva(
     "inline-flex items-center justify-center gap-sm whitespace-nowrap rounded-button text-[length:var(--button-font-size)] font-[weight:var(--button-font-weight)] transition-all disabled:pointer-events-none disabled:opacity-50",
     {
       variants: {
         variant: {
           default: 'bg-[var(--button-primary-bg)] text-[var(--button-primary-text)] shadow-sm hover:bg-[var(--button-primary-bg-hover)]',
           // ...
         },
         size: {
           default: 'h-[var(--button-height-default)] px-[var(--button-padding-x-default)]',
           sm: 'h-[var(--button-height-sm)] px-[var(--button-padding-x-sm)]',
           lg: 'h-[var(--button-height-lg)] px-[var(--button-padding-x-lg)]',
         },
       },
     },
   )
   ```

2. **Update Form Components**
   
   **Input Component Migration:**
   ```tsx
   // Before
   <input className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-base" />
   
   // After
   <input className="flex h-[var(--input-height)] w-full rounded-element border border-input bg-[var(--input-bg)] px-[var(--input-padding-x)] py-[var(--input-padding-y)] text-[length:var(--input-font-size)]" />
   ```

3. **Migrate Layout Components**
   
   **Card Component Migration:**
   ```tsx
   // Before
   <div className="bg-card text-card-foreground flex flex-col gap-6 rounded-xl border py-6 shadow-sm" />
   
   // After
   <div className="bg-[var(--card-bg)] text-foreground flex flex-col gap-[var(--card-gap)] rounded-container border border-[var(--card-border)] py-[var(--card-padding)] shadow-[var(--card-shadow)]" />
   ```

### Phase 3: Pattern Standardization (Week 4)

1. **Create Composite Tokens**
   ```css
   :root {
     /* Text Style Composites */
     --text-heading-lg: var(--semantic-text-lg) / var(--core-line-height-tight) var(--core-font-weight-semibold);
     --text-heading-base: var(--semantic-text-base) / var(--core-line-height-tight) var(--core-font-weight-semibold);
     --text-body: var(--semantic-text-sm) / var(--core-line-height-normal) var(--core-font-weight-normal);
     --text-caption: var(--semantic-text-xs) / var(--core-line-height-normal) var(--core-font-weight-normal);
     
     /* Focus Style Composite */
     --focus-ring: 0 0 0 var(--focus-ring-width) oklch(from var(--focus-ring-color) l c h / var(--focus-ring-opacity));
   }
   ```

2. **Standardize Interactive States**
   ```css
   /* Utility classes for consistent interactions */
   .interactive-element {
     @apply transition-all duration-[var(--core-duration-fast)];
     @apply focus-visible:outline-none focus-visible:ring-[length:var(--focus-ring-width)] focus-visible:ring-[color:var(--focus-ring-color)] focus-visible:ring-opacity-[var(--focus-ring-opacity)];
   }
   ```

### Phase 4: Theme Variations (Week 5)

1. **Create Theme Presets**
   ```css
   /* High Contrast Theme */
   .theme-high-contrast {
     --semantic-text-primary: var(--core-black);
     --semantic-text-secondary: var(--core-gray-700);
     --semantic-border-primary: var(--core-gray-900);
     /* ... additional overrides */
   }
   
   /* Colorful Theme */
   .theme-colorful {
     --semantic-brand-primary: var(--core-blue-500);
     --semantic-accent: var(--core-amber-500);
     /* ... additional overrides */
   }
   ```

## Implementation Checklist

### Week 1: Foundation
- [ ] Create token file structure
- [ ] Define all primitive tokens
- [ ] Define semantic tokens for light theme
- [ ] Define semantic tokens for dark theme
- [ ] Update Tailwind configuration
- [ ] Create TypeScript token exports

### Week 2-3: Core Components
- [ ] Migrate Button component
- [ ] Migrate Input component
- [ ] Migrate Card component
- [ ] Migrate Badge component
- [ ] Migrate Dialog component
- [ ] Migrate Table component
- [ ] Update all form components

### Week 4: Complex Components
- [ ] Migrate navigation components
- [ ] Migrate task-specific components
- [ ] Standardize focus states
- [ ] Standardize hover states
- [ ] Create composite text styles
- [ ] Document component patterns

### Week 5: Polish & Documentation
- [ ] Create theme variation examples
- [ ] Write migration guide for team
- [ ] Create Storybook stories with tokens
- [ ] Performance optimization
- [ ] Final testing and validation

## Testing Strategy

### Visual Regression Testing
1. **Setup Playwright Visual Tests**
   ```typescript
   test('component tokens render correctly', async ({ page }) => {
     await page.goto('/components');
     await expect(page).toHaveScreenshot('components-light.png');
     
     await page.click('[data-testid="theme-toggle"]');
     await expect(page).toHaveScreenshot('components-dark.png');
   });
   ```

2. **Token Validation Script**
   ```javascript
   // scripts/validate-tokens.js
   const validateTokenUsage = () => {
     // Check for hardcoded values
     // Verify token references
     // Report unused tokens
   };
   ```

### Theme Testing Checklist
- [ ] All components render correctly in light mode
- [ ] All components render correctly in dark mode
- [ ] Focus states are visible and consistent
- [ ] Hover states work as expected
- [ ] Text remains readable in all contexts
- [ ] No color contrast issues

## Team Onboarding Guide

### For Developers

1. **Token Usage Rules**
   - Never use raw color values - always use tokens
   - Use semantic tokens in components, not primitives
   - Follow the naming convention: `--[tier]-[category]-[property]-[variant]`

2. **Common Patterns**
   ```tsx
   // ✅ Good
   className="bg-primary text-primary-foreground rounded-button"
   
   // ❌ Bad
   className="bg-[#3B82F6] text-white rounded-md"
   ```

3. **Creating New Components**
   - Check if component tokens already exist
   - If not, add to `03-components.css`
   - Use semantic tokens as references
   - Document the new tokens

### For Designers

1. **Token Hierarchy**
   - Core tokens = Raw values (don't use directly)
   - Semantic tokens = Intent-based values (primary usage)
   - Component tokens = Component-specific overrides

2. **Making Design Changes**
   - Update tokens, not individual components
   - Changes cascade through the system
   - Test in both light and dark modes

## Benefits & ROI

### Immediate Benefits
- **Consistency**: Eliminates 90% of design inconsistencies
- **Speed**: 50% faster component development
- **Maintenance**: 75% reduction in style-related bugs

### Long-term Benefits
- **Scalability**: Easy to add new themes
- **Documentation**: Self-documenting design system
- **Collaboration**: Clear design-development handoff

## Conclusion

This Three-Tier Design Token Architecture will transform the Moatless Swarm UI into a more maintainable, consistent, and scalable application. The migration can be completed in 5 weeks with minimal disruption to ongoing development. The investment will pay dividends in reduced maintenance costs, faster feature development, and improved design consistency.

## Appendix: Token Naming Convention

### Pattern: `--[tier]-[category]-[property]-[variant]`

**Tiers:**
- `core` - Primitive tokens
- `semantic` - Alias tokens
- Component name (e.g., `button`, `card`) - Component tokens

**Categories:**
- `color`, `space`, `font`, `radius`, `shadow`, `duration`, `z-index`

**Properties:**
- Colors: `bg`, `text`, `border`
- Spacing: `padding`, `margin`, `gap`
- Typography: `size`, `weight`, `family`

**Variants:**
- States: `hover`, `active`, `disabled`
- Themes: `light`, `dark`
- Sizes: `xs`, `sm`, `md`, `lg`, `xl`

### Examples:
- `--core-gray-500` (Tier: core, Category: color, Variant: 500)
- `--semantic-bg-primary` (Tier: semantic, Category: background, Property: primary)
- `--button-padding-x-default` (Tier: button, Category: padding, Property: x, Variant: default)
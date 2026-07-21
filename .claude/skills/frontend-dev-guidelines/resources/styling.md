# Styling — Tailwind v4 + shadcn/ui

## Configuration

Tailwind v4 is configured in CSS — there is **no `tailwind.config.ts`**. Everything lives in `src/lib/styles/globals.css`:

```css
@import "tailwindcss";
@plugin "@tailwindcss/typography";
@custom-variant dark (&:is(.dark *));

:root {
  --background: oklch(1 0 0);
  --foreground: oklch(0.145 0 0);
  /* … full token set, incl. --success/--warning/--info and --status-* */
}
```

## Semantic tokens — the color rule

Always use shadcn semantic tokens; they adapt to dark mode automatically (no `dark:` classes in components):

- `bg-background` / `text-foreground` — page
- `bg-card` / `text-card-foreground` — surfaces
- `bg-muted` / `text-muted-foreground` — subtle
- `border-border`, `ring-ring` — UI elements
- `bg-primary` / `bg-secondary` / `bg-accent` / `bg-destructive` (+ `-foreground` pairs)
- Status/intent: `text-success`, `text-warning`, `text-info`; task/sandbox status via `text-status-*`

Never raw color scales (`bg-blue-600`, `text-gray-700`, `bg-white`) — they break dark mode and centralized theming. Need a new color? Add a token in `globals.css`.

## Class composition

Merge classes with `cn()` from `@/lib/utils` (clsx + tailwind-merge) — not raw `clsx`, not string concatenation:

```tsx
<div className={cn('flex items-center gap-2', isActive && 'bg-accent', className)} />
```

## Components

- Prefer shadcn/ui components from `@/lib/components/ui/`; add new ones with `cd apps/frontend && bunx shadcn@latest add [component]`.
- Mobile-first responsive design (`sm:` → `md:` → `lg:`).
- Keep custom CSS minimal; anything global goes in `globals.css`.

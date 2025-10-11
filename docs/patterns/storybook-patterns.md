# Storybook Patterns

AI-facing guidance for working with Storybook in this codebase.

## Core Goals
- Build UI in isolation while reusing the shared shadcn-based design system in `apps/frontend/src/lib/components/ui`
- Document component states so humans and agents can evaluate behaviour without running the full app
- Catch layout, theming, and interaction regressions before they hit production

## When to Create or Update Stories
- **Design system primitives** (buttons, inputs, cards, etc.) MUST have stories before merging. These components define our reusable shadcn surface; see `docs/patterns/schema-design-patterns.md` and `docs/patterns/component-design.md` for companion guidance.
- **Feature-level components** (chat panels, task layouts, selectors) SHOULD expose representative states whenever they render complex UI or depend on async data.
- **Bug fixes** that change visuals SHOULD update the relevant story (or add one) so the regression stays covered.
- **Removed components** MUST have their stories deleted in the same PR to avoid dead navigation entries.

## Directory & Naming Rules
```
apps/frontend/src/
└── lib/components/chat/chat-input/
    ├── chat-input.tsx
    └── __stories__/
        └── chat-input.stories.tsx
```
- Place stories in a sibling `__stories__/` directory.
- Match filenames to the component: `component-name.stories.tsx`.
- Keep stories close to the component so refactors stay atomic.

## Meta Configuration Pattern
```typescript
import type { Meta, StoryObj } from '@storybook/react'
import { SelectorButton } from '../selector-button'

const meta = {
  title: 'Chat/Selectors/SelectorButton',
  component: SelectorButton,
  parameters: {
    layout: 'centered', // 'centered' for primitives, 'fullscreen' for full views
  },
  decorators: [
    (Story) => (
      <div className="w-[400px] p-4 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SelectorButton>

export default meta

type Story = StoryObj<typeof meta>
```
**Rules:**
- Import `Meta` and `StoryObj` from `@storybook/react` (Vite builder).
- Use `satisfies Meta<typeof Component>` to keep type checking strict.
- Always choose an explicit `layout` so previews behave consistently.
- Keep decorators minimal and memo-friendly; wrap in a div rather than creating new providers unless required.
- Use `@/` aliases instead of relative paths when the component lives outside the local directory.

## Story Definitions
Prefer the args/controls API when the component exposes props you want agents to tweak, and fall back to a custom `render` when building multi-part layouts.

```typescript
export const Default: Story = {
  args: {
    label: 'Sonnet 4',
    icon: Brain,
    onClick: fn(), // use @storybook/test helpers for callbacks
  },
}

export const Disabled: Story = {
  args: {
    label: 'Disabled',
    disabled: true,
    onClick: fn(),
  },
}

export const MultipleSelectors: Story = {
  render: () => (
    <div className="flex gap-2">
      <SelectorButton icon={Brain} label="Sonnet 4" onClick={fn()} />
      <SelectorButton icon={Sparkles} label="Edit Mode" onClick={fn()} />
    </div>
  ),
}
```

### Variant Coverage Expectations
- Shadcn primitives MUST demonstrate default, hover/active (if visual), disabled, destructive, and any size variants.
- Feature components SHOULD capture loading, empty, success, and error or locked states.
- Use realistic copy and IDs; stories double as documentation for humans.

## Layout & Styling Context
- Global Tailwind styles load from `apps/frontend/src/lib/styles/globals.css` via `.storybook/preview.ts`. Do not re-import Tailwind in individual stories.
- Keep wrapper markup small (`div` with padding, width constraints, or viewport height). Tailwind tokens (`bg-background`, `text-muted-foreground`) come from the shadcn theme.
- When a component requires full-height context, wrap with a `h-screen` container and set `parameters.layout = 'fullscreen'`.

## Mock Data & Helpers
- Reuse existing mock utilities (e.g. chat message builders) when available. If you create new helpers, colocate them next to the story and export them for reuse if other stories need them.
- Use `fn()` from `@storybook/test` for noop callbacks so Chromatic/ERC tests can assert invocations if play functions are added later.
- Freeze dates with `new Date('2024-01-01T00:00:00Z')` when the timestamp matters visually; otherwise use helper builders to set them consistently.

## Play Functions & Interactions
Use Storybook play functions to showcase interaction flows when it adds clarity:
```typescript
import { userEvent, within } from '@storybook/test'

export const OpensDialog: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: 'Open dialog' }))
  },
}
```
- Play steps SHOULD auto-open modals, dropdowns, or validation messages so reviewers immediately see the important state.
- Keep interactions deterministic and prefer user-visible assertions (buttons, labels, roles).

## Working With the Design System
- All UI primitives come from the shadcn-based library in `apps/frontend/src/lib/components/ui`. Stories for these components drive trust in the design system.
- Align props and variants with the guidance in `docs/patterns/component-design.md`; the same naming and SRP expectations apply to story composition.
- When wrapping primitives in feature components, create a story at the feature level (`Features/Tasks/...`) and reference the primitive stories for additional styling inspiration instead of duplicating them.

## Running Storybook Locally
- Use `bun run storybook` (from repo root) to start Storybook on port 6006.
- Use `bun run build-storybook` to generate static documentation; run locally before pushing major UI changes to ensure builds succeed.
- Chromatic relies on the same stories, so keep CI-friendly (deterministic) data and avoid random IDs.

## Implementation Checklist
1. **Placement** – Story lives in `__stories__/` next to the component with the correct filename.
2. **Meta** – Title follows `Domain/Subdomain/Component`, `layout` is set, `satisfies Meta<typeof Component>` is present.
3. **Coverage** – Default and edge variants are represented with realistic data.
4. **Decorators** – Only the minimum wrappers required for layout; no ad-hoc provider trees without justification.
5. **Mocks** – Use `fn()` for callbacks and share mock builders when logic repeats.
6. **Verification** – Story renders without runtime warnings when running `bun run storybook`.

## Anti-Patterns to Avoid
- **Flat titles** like `'Button'` – always provide a two-level hierarchy (`UI/Button`).
- **Stories in component folders** – keep them inside `__stories__/` to avoid bundling issues.
- **Anonymous callbacks** like `() => {}` – use `fn()` to stay inspectable in tests.
- **Manual CSS imports** – rely on the global import in `.storybook/preview.ts`.
- **Randomized data** – deterministic seeds only; random numbers break Chromatic diffs.

Follow these patterns and every agent (human or AI) can trust Storybook as the UI contract for this workspace.

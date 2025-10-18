# Feature Flag Pattern

## Overview

Feature flags control application features using environment variables with the `FF_` prefix. They enable runtime feature toggling without code changes.

### When to Use Feature Flags

- MUST use for features that need runtime toggling
- SHOULD use for deployment-specific configurations
- SHOULD use for experimental or beta features
- MUST NOT use as a replacement for proper configuration management

## Adding New Feature Flags

### Step 1: Define Flag Constant

**File:** `packages/types/src/constants/feature-flags.ts`

```typescript
// CORRECT
export const FEATURE_FLAGS = {
  NETLIFY_PUBLISH: 'NETLIFY_PUBLISH',
  SHOW_SYSTEM_SETTINGS: 'SHOW_SYSTEM_SETTINGS',
  NEW_FEATURE: 'NEW_FEATURE', // Add your flag here
} as const
```

- MUST use SCREAMING_SNAKE_CASE for flag names
- MUST add descriptive JSDoc comment above the flag
- MUST keep flag names concise but clear

### Step 2: Set Default Value

**Same file:** `packages/types/src/constants/feature-flags.ts`

```typescript
// CORRECT
export const FEATURE_FLAG_DEFAULTS: Record<FeatureFlagKey, boolean> = {
  NETLIFY_PUBLISH: false,        // Disabled by default
  SHOW_SYSTEM_SETTINGS: true,    // Enabled by default
  NEW_FEATURE: false,            // Add default here
}
```

- MUST set a default value for every flag
- SHOULD default to `false` for new/experimental features
- SHOULD default to `true` for core functionality that can be disabled

### Step 3: Update Response Schema

**File:** `packages/types/src/schemas/feature-flags.schemas.ts`

```typescript
// CORRECT
export const FeatureFlagsResponseSchema = z.object({
  netlify_publish: z.boolean().describe('Whether Netlify publishing features are enabled'),
  show_system_settings: z.boolean().describe('Whether system configuration sections are visible'),
  new_feature: z.boolean().describe('Description of what this flag controls'),
})
```

- MUST use lowercase_snake_case for schema keys
- MUST add descriptive `.describe()` text
- MUST match the flag name pattern (FLAG_NAME → flag_name)

### Step 4: Update Frontend Fallback

**File:** `apps/frontend/src/lib/contexts/feature-flags-context.tsx`

```typescript
// CORRECT - Update error fallback with new flag
setFlags({
  netlify_publish: false,
  show_system_settings: true,
  new_feature: false, // Add with same default as backend
})
```

- MUST add new flag to error fallback object
- MUST use the same default value as backend

### Step 5: Document in .env.example

**File:** `.env.example`

```bash
# CORRECT
# Enable new feature (default: false)
# When enabled, shows/enables [describe what happens]
# FF_NEW_FEATURE=true
```

- MUST add comment block for the new flag
- MUST specify the default value
- MUST describe what the flag controls
- SHOULD include usage examples if behavior is complex

## Using Feature Flags

### Backend Usage

**File:** Any backend service or route

```typescript
// CORRECT
import { isFeatureEnabled } from '../utils/feature-flags.js'

if (isFeatureEnabled('NEW_FEATURE')) {
  // Feature is enabled
  return enhancedResponse
}
// Feature is disabled
return standardResponse
```

**Available function:**
- `isFeatureEnabled(flag: FeatureFlagKey): boolean` - Check if flag is enabled

### Frontend Usage

**File:** Any React component

```typescript
// CORRECT
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'

export function MyComponent() {
  const isNewFeatureEnabled = useFeatureFlag('NEW_FEATURE')

  if (!isNewFeatureEnabled) {
    return null // Hide feature
  }

  return <NewFeatureComponent />
}
```

**Conditional Rendering Patterns:**

```typescript
// CORRECT - Conditional section in array
const sections = useMemo(() => {
  const allSections = []

  if (useFeatureFlag('NEW_FEATURE')) {
    allSections.push({
      id: 'new-section',
      component: <NewSection />,
    })
  }

  return allSections
}, [isNewFeatureEnabled])

// CORRECT - Ternary for single element
<div>
  {isNewFeatureEnabled ? <NewFeature /> : <OldFeature />}
</div>

// CORRECT - Null coalescing for optional rendering
<div>
  {isNewFeatureEnabled && <OptionalFeature />}
</div>
```

### Common Mistakes

```typescript
// WRONG - Hardcoded flag name
const isEnabled = useFeatureFlag('new_feature') // lowercase won't work

// CORRECT - Use constant
const isEnabled = useFeatureFlag('NEW_FEATURE')

// WRONG - Direct env var access in frontend
if (import.meta.env.FF_NEW_FEATURE) { } // Won't work

// CORRECT - Use the hook
if (useFeatureFlag('NEW_FEATURE')) { }

// WRONG - Missing hook dependency
const sections = useMemo(() => {
  // Using flag but not in dependency array
}, []) // Missing isNewFeatureEnabled

// CORRECT - Include in dependencies
const sections = useMemo(() => {
  // Using flag
}, [isNewFeatureEnabled])
```

## Implementation Checklist

Before committing feature flag changes:

1. **Flag Definition**
   - [ ] Added to `FEATURE_FLAGS` constant with SCREAMING_SNAKE_CASE
   - [ ] Added to `FEATURE_FLAG_DEFAULTS` with appropriate default
   - [ ] Added JSDoc comment describing the flag's purpose

2. **Schema Updates**
   - [ ] Added to `FeatureFlagsResponseSchema` with lowercase_snake_case
   - [ ] Added descriptive `.describe()` text
   - [ ] Updated frontend error fallback with matching default

3. **Documentation**
   - [ ] Added to `.env.example` with usage instructions
   - [ ] Specified default value in comment
   - [ ] Described what the flag controls

4. **Testing**
   - [ ] Run `bun run type:check` - TypeScript types valid
   - [ ] Run `bun run biome:check` - Linting passes
   - [ ] Test with flag enabled: `FF_NEW_FEATURE=true bun run dev`
   - [ ] Test with flag disabled: `FF_NEW_FEATURE=false bun run dev`
   - [ ] Test default behavior (no env var set)

5. **Code Usage**
   - [ ] Backend uses `isFeatureEnabled('FLAG_NAME')`
   - [ ] Frontend uses `useFeatureFlag('FLAG_NAME')`
   - [ ] No hardcoded flag name strings (use constants)
   - [ ] All conditional sections include flag in `useMemo`/`useEffect` dependencies

6. **Verification**
   - [ ] Flag works as expected when enabled
   - [ ] Flag works as expected when disabled
   - [ ] No TypeScript errors
   - [ ] No runtime errors in console

## Examples from Codebase

### Example 1: NETLIFY_PUBLISH
**Purpose:** Controls visibility of Netlify publishing features

**Frontend usage:**
```typescript
// apps/frontend/src/features/settings/components/settings-dialog.tsx
const isNetlifyPublishEnabled = useFeatureFlag('NETLIFY_PUBLISH')

if (isNetlifyPublishEnabled) {
  allSections.push({
    id: 'publish',
    label: 'Publish',
    component: <PublishConfigurationSection />,
  })
}
```

```typescript
// apps/frontend/src/features/preview/components/preview-header.tsx
const isNetlifyPublishEnabled = useFeatureFlag('NETLIFY_PUBLISH')

<div>
  {isNetlifyPublishEnabled ? (
    <PublishButton {...props} />
  ) : null}
</div>
```

**Environment variable:**
```bash
FF_NETLIFY_PUBLISH=true  # Show publish features
```

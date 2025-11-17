# Trigger Patterns Reference

Complete guide to all trigger types with real examples from this project's skills.

---

## Table of Contents

- [Overview](#overview)
- [Keyword Triggers](#keyword-triggers)
- [Intent Pattern Triggers](#intent-pattern-triggers)
- [File Path Triggers](#file-path-triggers)
- [Content Pattern Triggers](#content-pattern-triggers)
- [Combining Triggers](#combining-triggers)
- [Testing Triggers](#testing-triggers)
- [Best Practices](#best-practices)

---

## Overview

Skills activate based on trigger patterns defined in `.claude/skills/skill-rules.json`. There are two main trigger types:

1. **Prompt Triggers** - Based on what the user says
   - Keywords (explicit mentions)
   - Intent patterns (implicit actions)

2. **File Triggers** - Based on files being edited
   - Path patterns (file location)
   - Content patterns (file contents)
   - Path exclusions (files to ignore)

---

## Keyword Triggers

### What They Are

Keywords are explicit terms that, when mentioned in a user's prompt, trigger skill activation.

- Case-insensitive matching
- Exact word matching (not substring)
- Single words or short phrases

### Configuration

```json
{
  "promptTriggers": {
    "keywords": ["component", "react", "frontend", "UI", "page"]
  }
}
```

### Real Examples from This Project

**Frontend Skill Keywords:**
```json
"keywords": [
  "component",
  "react",
  "frontend",
  "UI",
  "page",
  "tanstack",
  "query",
  "router",
  "tailwind",
  "shadcn",
  "form",
  "modal",
  "dialog",
  "useQuery",
  "useMutation",
  "createFileRoute",
  "styling",
  "performance",
  "optimization"
]
```

**Backend Skill Keywords:**
```json
"keywords": [
  "backend",
  "API",
  "endpoint",
  "route",
  "service",
  "hono",
  "inversify",
  "drizzle",
  "repository",
  "schema",
  "validation",
  "zod",
  "database",
  "OpenAPI",
  "contract",
  "buildPath",
  "createRoute",
  "dependency injection"
]
```

**Testing Skill Keywords:**
```json
"keywords": [
  "test",
  "testing",
  "spec",
  "vitest",
  "coverage",
  "mock",
  "unit test",
  "integration test",
  "test suite",
  "describe",
  "it",
  "expect"
]
```

### User Prompts That Trigger

**Frontend Skill:**
- "Create a new component for task list"
- "Add a modal dialog to the settings page"
- "How do I use useQuery to fetch data?"
- "Optimize the performance of this React component"

**Backend Skill:**
- "Create a new API endpoint for tasks"
- "Add validation schema for user registration"
- "Set up dependency injection for the service"
- "How do I use Drizzle ORM to query the database?"

**Testing Skill:**
- "Write unit tests for the TaskService"
- "Add coverage for the authentication flow"
- "How do I mock external dependencies?"

### Best Practices

- ✅ Include technology names (react, hono, vitest)
- ✅ Include action verbs (create, fetch, optimize)
- ✅ Include domain terms (component, endpoint, test)
- ✅ Include specific APIs (useQuery, createRoute)
- ❌ Don't use overly generic terms (code, file, function)
- ❌ Don't duplicate keywords across skills

---

## Intent Pattern Triggers

### What They Are

Intent patterns are regex patterns that detect implicit user intentions based on action verbs and objects.

- Regex-based matching
- Capture action + object combinations
- More flexible than keywords

### Configuration

```json
{
  "promptTriggers": {
    "intentPatterns": [
      "(create|add|build).*?(component|page|UI)",
      "(fetch|load).*?data"
    ]
  }
}
```

### Regex Pattern Syntax

**Basic structure:**
```regex
(action1|action2|action3).*?(object1|object2)
```

- `(create|add|build)` - Match any of these verbs
- `.*?` - Match any characters (non-greedy)
- `(component|page|UI)` - Match any of these objects

**Common regex elements:**
- `.*?` - Any characters, non-greedy
- `\s+` - One or more spaces
- `\b` - Word boundary
- `[a-z]+` - One or more lowercase letters
- `?` - Optional (0 or 1)

### Real Examples from This Project

**Frontend Skill Intent Patterns:**
```json
"intentPatterns": [
  "(create|add|build).*?(component|page|UI|modal|dialog)",
  "(fetch|load).*?data",
  "(style|design).*?(component|UI)",
  "(optimize|improve).*?(performance|rendering)"
]
```

**Backend Skill Intent Patterns:**
```json
"intentPatterns": [
  "(create|add).*?(route|endpoint|API|service)",
  "(implement|add).*?(validation|middleware)",
  "(database|data).*?(access|query|operation)",
  "(define|create).*?(schema|contract)"
]
```

**Testing Skill Intent Patterns:**
```json
"intentPatterns": [
  "(write|create|add).*?test",
  "(test|mock).*?(component|service|function|hook)",
  "(add|increase).*?coverage"
]
```

### User Prompts That Match

**Frontend Skill:**
- "I need to create a modal for user settings" → `(create|add|build).*?(modal)`
- "Let's fetch data from the API" → `(fetch|load).*?data`
- "Can you style this component?" → `(style|design).*?(component)`
- "Improve the rendering performance" → `(optimize|improve).*?(rendering)`

**Backend Skill:**
- "Add an endpoint for user registration" → `(create|add).*?(endpoint)`
- "Implement validation for the request body" → `(implement|add).*?(validation)`
- "Query the database for tasks" → `(database).*?(query)`
- "Create a schema for the Task resource" → `(create).*?(schema)`

**Testing Skill:**
- "Write tests for the TaskService" → `(write|create|add).*?test`
- "Mock the API client" → `(test|mock).*?(?!service)` (matches "API client")
- "Increase test coverage" → `(add|increase).*?coverage`

### Best Practices

- ✅ Use non-greedy matching (`.*?` not `.*`)
- ✅ Include multiple action verbs
- ✅ Include multiple target objects
- ✅ Test patterns with real user prompts
- ✅ Keep patterns focused (avoid matching too broadly)
- ❌ Don't use overly greedy patterns that match everything
- ❌ Don't forget word boundaries when needed
- ❌ Don't make patterns too complex (reduces readability)

---

## File Path Triggers

### What They Are

Path patterns are glob patterns that match file paths. Skills activate when files matching these patterns are edited.

- Glob syntax (not regex)
- Match file locations in the project
- Support wildcards and recursive matching

### Configuration

```json
{
  "fileTriggers": {
    "pathPatterns": [
      "apps/frontend/src/**/*.tsx",
      "apps/backend/src/**/*.ts"
    ],
    "pathExclusions": [
      "**/*.test.*",
      "**/__tests__/**"
    ]
  }
}
```

### Glob Pattern Syntax

**Wildcards:**
- `*` - Match any characters except `/`
- `**` - Match any characters including `/` (recursive)
- `?` - Match single character
- `[abc]` - Match any character in set
- `{a,b}` - Match either pattern

**Examples:**
- `*.ts` - All TypeScript files in current directory
- `**/*.tsx` - All TSX files recursively
- `src/**/*.{ts,tsx}` - All TS and TSX files in src/
- `apps/*/src/**/*.ts` - All TS files in any app's src/

### Real Examples from This Project

**Frontend Skill Path Patterns:**
```json
"pathPatterns": [
  "apps/frontend/src/**/*.tsx",
  "apps/frontend/src/**/*.ts"
],
"pathExclusions": [
  "**/*.test.*",
  "**/__tests__/**"
]
```

**Matches:**
- `apps/frontend/src/features/tasks/components/task-list.tsx` ✅
- `apps/frontend/src/routes/tasks.tsx` ✅
- `apps/frontend/src/lib/hooks/use-tasks.ts` ✅

**Excludes:**
- `apps/frontend/src/features/tasks/components/task-list.test.tsx` ❌
- `apps/frontend/src/__tests__/setup.ts` ❌

**Backend Skill Path Patterns:**
```json
"pathPatterns": [
  "apps/backend/src/**/*.ts",
  "packages/types/src/**/*.ts",
  "packages/api-contract/src/**/*.ts"
],
"pathExclusions": [
  "**/*.test.*",
  "**/__tests__/**"
]
```

**Matches:**
- `apps/backend/src/routes/tasks.ts` ✅
- `apps/backend/src/services/task.service.ts` ✅
- `packages/types/src/schemas/task.schema.ts` ✅
- `packages/api-contract/src/routes/tasks.ts` ✅

**Testing Skill Path Patterns:**
```json
"pathPatterns": [
  "**/*.test.ts",
  "**/*.test.tsx",
  "**/*.integration.test.ts",
  "**/__tests__/**/*"
]
```

**Matches:**
- `apps/frontend/src/features/tasks/__tests__/task-list.test.tsx` ✅
- `apps/backend/src/services/task.service.test.ts` ✅
- `apps/backend/src/repositories/task.repository.integration.test.ts` ✅

### Best Practices

- ✅ Use `**` for recursive directory matching
- ✅ Include all relevant directories for the skill
- ✅ Use `pathExclusions` to exclude test files (for non-testing skills)
- ✅ Test patterns against actual file paths in the project
- ✅ Be specific enough to avoid false positives
- ❌ Don't make patterns too broad (e.g., `**/*`)
- ❌ Don't forget to exclude test files for production code skills
- ❌ Don't overlap patterns across skills unnecessarily

---

## Content Pattern Triggers

### What They Are

Content patterns are regex patterns that match file contents. Skills activate when editing files containing these patterns.

- Regex-based matching
- Search file content, not just path
- Useful for technology-specific detection

### Configuration

```json
{
  "fileTriggers": {
    "contentPatterns": [
      "import.*react",
      "useQuery|useMutation",
      "from '@tanstack/(query|router)'"
    ]
  }
}
```

### Regex Pattern Syntax

**Common patterns:**
- `import.*react` - Import statements with "react"
- `from 'package'` - Imports from specific package
- `@decorator` - Decorator usage
- `className.*=` - JSX className attributes

**Escaping:**
- Use `\\` to escape special regex characters
- `\\.` - Literal dot
- `\\(` - Literal parenthesis
- `\\[` - Literal bracket

### Real Examples from This Project

**Frontend Skill Content Patterns:**
```json
"contentPatterns": [
  "import.*react",
  "useQuery|useMutation|useSuspenseQuery",
  "createFileRoute",
  "from '@tanstack/(query|router)'",
  "className.*="
]
```

**Matches files containing:**
- `import React from 'react'`
- `import { useQuery } from '@tanstack/react-query'`
- `const { data } = useQuery({ ... })`
- `export const Route = createFileRoute('/tasks')(...)`
- `<div className="flex gap-2">`

**Backend Skill Content Patterns:**
```json
"contentPatterns": [
  "OpenAPIHono",
  "@injectable|@inject",
  "createRoute",
  "z\\.(object|string|number|array)",
  "from 'inversify'",
  "from '@moatless/api-contract'"
]
```

**Matches files containing:**
- `const app = new OpenAPIHono()`
- `@injectable()` or `@inject(TYPES.TaskRepository)`
- `export const route = createRoute({ ... })`
- `z.object({ ... })` or `z.string()`
- `import { injectable } from 'inversify'`
- `import { routes } from '@moatless/api-contract'`

### Best Practices

- ✅ Match technology-specific imports
- ✅ Match decorators or annotations
- ✅ Match framework-specific patterns
- ✅ Use alternation (`|`) for multiple options
- ✅ Escape special regex characters
- ❌ Don't make patterns too generic
- ❌ Don't forget to escape special characters
- ❌ Don't rely only on content patterns (combine with path patterns)

---

## Combining Triggers

Skills can use multiple trigger types together. Claude Code activates a skill when:
- **Prompt triggers**: Keywords OR intent patterns match
- **File triggers**: Path patterns AND content patterns match (if both specified)

### Example: Frontend Skill

```json
{
  "frontend-dev-guidelines": {
    "promptTriggers": {
      "keywords": ["component", "react", "UI"],
      "intentPatterns": ["(create|add).*?(component|page)"]
    },
    "fileTriggers": {
      "pathPatterns": ["apps/frontend/src/**/*.tsx"],
      "pathExclusions": ["**/*.test.*"],
      "contentPatterns": ["import.*react"]
    }
  }
}
```

**Activates when:**
- User says "create a component" (keyword + intent) OR
- User edits `apps/frontend/src/features/tasks/task-list.tsx` (path) that contains `import React` (content)

### Trigger Logic

```
Skill activates IF:
  (keyword match OR intent pattern match)
  OR
  (path pattern match AND content pattern match AND NOT path exclusion match)
```

---

## Testing Triggers

### Manual Testing

Test UserPromptSubmit hook with different prompts:

```bash
cd .claude/hooks

# Test frontend skill triggers
echo '{"session_id":"test","prompt":"create a new React component"}' | npx tsx skill-activation-prompt.ts

# Test backend skill triggers
echo '{"session_id":"test","prompt":"add an API endpoint for users"}' | npx tsx skill-activation-prompt.ts

# Test testing skill triggers
echo '{"session_id":"test","prompt":"write unit tests for TaskService"}' | npx tsx skill-activation-prompt.ts
```

**Expected output:**
- Skill reminder injected if triggers match
- No output if no triggers match

### Testing Path Patterns

Create a test file matching the pattern:

```bash
# Test frontend path pattern
touch apps/frontend/src/test-component.tsx
# Edit this file and verify frontend skill activates

# Test backend path pattern
touch apps/backend/src/test-service.ts
# Edit this file and verify backend skill activates
```

### Testing Content Patterns

Create a file with matching content:

```bash
# Test React import detection
echo "import React from 'react'" > test.tsx
# Edit this file in a matching path and verify skill activates

# Test Hono detection
echo "const app = new OpenAPIHono()" > test.ts
# Edit this file in a matching path and verify skill activates
```

### Validation Checklist

When testing triggers:
- [ ] Test with multiple user prompts
- [ ] Test keyword variations
- [ ] Test intent pattern variations
- [ ] Test path patterns with real file paths
- [ ] Test content patterns with real file contents
- [ ] Test path exclusions work correctly
- [ ] Verify no false positives
- [ ] Verify no false negatives
- [ ] Test with files from different project areas

---

## Best Practices

### General Guidelines

1. **Be Specific**: Avoid overly broad patterns that trigger too often
2. **Be Complete**: Include all relevant keywords and patterns
3. **Test Thoroughly**: Use real user prompts and file paths
4. **Avoid Overlap**: Don't duplicate triggers across skills unnecessarily
5. **Document Intent**: Comment complex patterns in skill-rules.json

### Keyword Best Practices

- Include technology names
- Include action verbs
- Include domain concepts
- Include specific APIs
- Avoid generic terms

### Intent Pattern Best Practices

- Use non-greedy matching
- Include multiple verbs
- Include multiple objects
- Test with real prompts
- Keep focused

### Path Pattern Best Practices

- Use `**` for recursion
- Include all relevant directories
- Use exclusions for test files
- Test with real paths
- Be specific

### Content Pattern Best Practices

- Match imports
- Match decorators
- Match framework patterns
- Escape special characters
- Combine with path patterns

---

## Common Pitfalls

### Pitfall 1: Overly Broad Patterns

**Problem:**
```json
"intentPatterns": ["(create|add|implement).*"]
```
Matches everything with "create", "add", or "implement".

**Solution:**
```json
"intentPatterns": ["(create|add|implement).*?(component|service|test)"]
```
Requires specific object after action.

### Pitfall 2: Missing Escapes

**Problem:**
```json
"contentPatterns": ["z.(object|string)"]
```
`.` matches any character, not literal dot.

**Solution:**
```json
"contentPatterns": ["z\\.(object|string)"]
```
Escape the dot.

### Pitfall 3: Too Specific Exclusions

**Problem:**
```json
"pathExclusions": ["apps/frontend/src/__tests__/**"]
```
Only excludes tests in one specific directory.

**Solution:**
```json
"pathExclusions": ["**/__tests__/**", "**/*.test.*"]
```
Excludes all test files and directories.

### Pitfall 4: Greedy Matching

**Problem:**
```json
"intentPatterns": ["create.*component"]
```
`.*` is greedy, matches too much.

**Solution:**
```json
"intentPatterns": ["create.*?component"]
```
`.*?` is non-greedy.

---

## Summary

### Trigger Types

| Type | Syntax | Use Case | Example |
|------|--------|----------|---------|
| Keywords | Simple strings | Explicit mentions | `"component"`, `"API"` |
| Intent Patterns | Regex | Action detection | `"(create\|add).*?component"` |
| Path Patterns | Glob | File location | `"apps/frontend/**/*.tsx"` |
| Content Patterns | Regex | Technology detection | `"import.*react"` |

### Quick Reference

**Create effective triggers:**
1. Start with keywords (simple, explicit)
2. Add intent patterns (flexible, action-based)
3. Add path patterns (file location)
4. Add content patterns (technology-specific)
5. Test thoroughly with real examples
6. Refine based on false positives/negatives

**Test triggers:**
```bash
# UserPromptSubmit
echo '{"prompt":"test prompt"}' | npx tsx .claude/hooks/skill-activation-prompt.ts

# Validate JSON
jq . .claude/skills/skill-rules.json
```

---

**This reference provides all the patterns and examples needed to create effective skill triggers for Claude Code.**

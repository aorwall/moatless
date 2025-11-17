# Skill Rules Reference

Complete JSON schema documentation for `.claude/skills/skill-rules.json`.

---

## Table of Contents

- [File Location and Purpose](#file-location-and-purpose)
- [Top-Level Structure](#top-level-structure)
- [Skill Configuration Schema](#skill-configuration-schema)
- [Field Reference](#field-reference)
- [Complete Examples](#complete-examples)
- [Validation](#validation)
- [Common Errors](#common-errors)

---

## File Location and Purpose

**Location:** `.claude/skills/skill-rules.json`

**Purpose:**
- Define all skills in the project
- Configure trigger patterns for skill activation
- Set enforcement levels and priorities
- Configure skip conditions

**Format:** JSON (strictly validated)

---

## Top-Level Structure

```json
{
  "version": "1.0",
  "description": "Optional description of the skill system",
  "skills": {
    "skill-name-1": { /* skill configuration */ },
    "skill-name-2": { /* skill configuration */ },
    "skill-name-3": { /* skill configuration */ }
  }
}
```

### Top-Level Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `version` | string | Yes | Schema version (currently "1.0") |
| `description` | string | No | Description of the skill system |
| `skills` | object | Yes | Map of skill name to configuration |

---

## Skill Configuration Schema

Each skill in the `skills` object uses this structure:

```json
{
  "skill-name": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "high",
    "description": "Brief description...",
    "promptTriggers": {
      "keywords": ["keyword1", "keyword2"],
      "intentPatterns": ["regex1", "regex2"]
    },
    "fileTriggers": {
      "pathPatterns": ["glob1", "glob2"],
      "pathExclusions": ["glob3", "glob4"],
      "contentPatterns": ["regex3", "regex4"]
    },
    "skipConditions": {
      "sessionSkillUsed": true
    }
  }
}
```

---

## Field Reference

### `type`

**Type:** string
**Required:** Yes
**Values:** `"domain"`

Defines the skill type. This project uses only domain skills.

**Example:**
```json
"type": "domain"
```

---

### `enforcement`

**Type:** string
**Required:** Yes
**Values:** `"suggest"`

Defines how the skill is enforced. This project uses only suggest mode.

**Example:**
```json
"enforcement": "suggest"
```

**Enforcement Modes:**
- `"suggest"` - Advisory mode, skill reminder injected via UserPromptSubmit hook

---

### `priority`

**Type:** string
**Required:** Yes
**Values:** `"high"`, `"medium"`, `"low"`

Defines the priority level for skill activation.

**Example:**
```json
"priority": "high"
```

**Priority Levels:**
- `"high"` - Core development skills (frontend, backend, testing)
- `"medium"` - Specialized skills (e2e-testing, skill-developer)
- `"low"` - Optional or informational skills

---

### `description`

**Type:** string
**Required:** No
**Max Length:** 1024 characters

Brief description of what the skill covers. Useful for documentation.

**Example:**
```json
"description": "React 19 + TanStack Query/Router development patterns with Tailwind v4 and shadcn/ui components"
```

---

### `promptTriggers`

**Type:** object
**Required:** No (but recommended)

Defines patterns that trigger skill activation based on user prompts.

**Structure:**
```json
"promptTriggers": {
  "keywords": ["string", ...],
  "intentPatterns": ["regex", ...]
}
```

#### `promptTriggers.keywords`

**Type:** array of strings
**Required:** No

List of keywords that trigger skill activation when mentioned in user prompts.

- Case-insensitive matching
- Exact word matching (not substring)
- Single words or short phrases

**Example:**
```json
"keywords": [
  "component",
  "react",
  "frontend",
  "UI",
  "page",
  "tanstack",
  "query",
  "router"
]
```

#### `promptTriggers.intentPatterns`

**Type:** array of regex strings
**Required:** No

List of regex patterns that detect user intent in prompts.

- Regex syntax (JavaScript flavor)
- Capture action + object combinations
- Use non-greedy matching (`.*?`)

**Example:**
```json
"intentPatterns": [
  "(create|add|build).*?(component|page|UI)",
  "(fetch|load).*?data",
  "(optimize|improve).*?(performance|rendering)"
]
```

---

### `fileTriggers`

**Type:** object
**Required:** No

Defines patterns that trigger skill activation based on files being edited.

**Structure:**
```json
"fileTriggers": {
  "pathPatterns": ["glob", ...],
  "pathExclusions": ["glob", ...],
  "contentPatterns": ["regex", ...]
}
```

#### `fileTriggers.pathPatterns`

**Type:** array of glob patterns
**Required:** No (if using fileTriggers)

List of glob patterns matching file paths that should trigger skill activation.

- Glob syntax (not regex)
- Use `**` for recursive matching
- Use `*` for single-level wildcards

**Example:**
```json
"pathPatterns": [
  "apps/frontend/src/**/*.tsx",
  "apps/frontend/src/**/*.ts"
]
```

#### `fileTriggers.pathExclusions`

**Type:** array of glob patterns
**Required:** No

List of glob patterns for files to exclude from skill activation.

**Example:**
```json
"pathExclusions": [
  "**/*.test.*",
  "**/__tests__/**"
]
```

#### `fileTriggers.contentPatterns`

**Type:** array of regex strings
**Required:** No

List of regex patterns that match file contents. Skill activates when editing files containing these patterns.

- Regex syntax (JavaScript flavor)
- Match imports, decorators, framework patterns
- Escape special characters with `\\`

**Example:**
```json
"contentPatterns": [
  "import.*react",
  "useQuery|useMutation|useSuspenseQuery",
  "createFileRoute",
  "from '@tanstack/(query|router)'"
]
```

---

### `skipConditions`

**Type:** object
**Required:** No

Defines conditions under which skill activation should be skipped.

**Structure:**
```json
"skipConditions": {
  "sessionSkillUsed": boolean
}
```

#### `skipConditions.sessionSkillUsed`

**Type:** boolean
**Required:** No
**Default:** false

If `true`, skill will not trigger again in the same session after being suggested once.

**Example:**
```json
"skipConditions": {
  "sessionSkillUsed": true
}
```

**Behavior:**
- First trigger in session → Skill suggested
- Subsequent triggers → Skill not suggested (session tracking)
- New session → Skill can trigger again

**Session State:** Stored in `.claude/hooks/state/skills-used-{session_id}.json`

---

## Complete Examples

### Example 1: Frontend Development Skill

```json
{
  "frontend-dev-guidelines": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "high",
    "description": "React 19 + TanStack Query/Router development patterns with Tailwind v4 and shadcn/ui components",
    "promptTriggers": {
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
      ],
      "intentPatterns": [
        "(create|add|build).*?(component|page|UI|modal|dialog)",
        "(fetch|load).*?data",
        "(style|design).*?(component|UI)",
        "(optimize|improve).*?(performance|rendering)"
      ]
    },
    "fileTriggers": {
      "pathPatterns": [
        "apps/frontend/src/**/*.tsx",
        "apps/frontend/src/**/*.ts"
      ],
      "pathExclusions": [
        "**/*.test.*",
        "**/__tests__/**"
      ],
      "contentPatterns": [
        "import.*react",
        "useQuery|useMutation|useSuspenseQuery",
        "createFileRoute",
        "from '@tanstack/(query|router)'",
        "className.*="
      ]
    },
    "skipConditions": {
      "sessionSkillUsed": true
    }
  }
}
```

### Example 2: Backend Development Skill

```json
{
  "backend-dev-guidelines": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "high",
    "description": "Hono + InversifyJS + Drizzle backend patterns with OpenAPI-first contract design",
    "promptTriggers": {
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
      ],
      "intentPatterns": [
        "(create|add).*?(route|endpoint|API|service)",
        "(implement|add).*?(validation|middleware)",
        "(database|data).*?(access|query|operation)",
        "(define|create).*?(schema|contract)"
      ]
    },
    "fileTriggers": {
      "pathPatterns": [
        "apps/backend/src/**/*.ts",
        "packages/types/src/**/*.ts",
        "packages/api-contract/src/**/*.ts"
      ],
      "pathExclusions": [
        "**/*.test.*",
        "**/__tests__/**"
      ],
      "contentPatterns": [
        "OpenAPIHono",
        "@injectable|@inject",
        "createRoute",
        "z\\.(object|string|number|array)",
        "from 'inversify'",
        "from '@moatless/api-contract'"
      ]
    },
    "skipConditions": {
      "sessionSkillUsed": true
    }
  }
}
```

### Example 3: Testing Skill

```json
{
  "testing-guidelines": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "high",
    "description": "Vitest testing patterns with tier classification and strategic mocking",
    "promptTriggers": {
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
      ],
      "intentPatterns": [
        "(write|create|add).*?test",
        "(test|mock).*?(component|service|function|hook)",
        "(add|increase).*?coverage"
      ]
    },
    "fileTriggers": {
      "pathPatterns": [
        "**/*.test.ts",
        "**/*.test.tsx",
        "**/*.integration.test.ts",
        "**/__tests__/**/*"
      ]
    },
    "skipConditions": {
      "sessionSkillUsed": true
    }
  }
}
```

### Example 4: Specialized Skill (E2E Testing)

```json
{
  "e2e-testing": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "medium",
    "description": "End-to-end testing patterns for the Moatless Vibe application",
    "promptTriggers": {
      "keywords": [
        "e2e",
        "end-to-end",
        "integration test",
        "workflow test",
        "ui test",
        "playwright"
      ],
      "intentPatterns": [
        "(test|verify).*?(workflow|UI|integration)",
        "(add|create).*?e2e.*?test"
      ]
    },
    "fileTriggers": {
      "pathPatterns": [
        ".claude/skills/e2e-testing/**/*"
      ]
    }
  }
}
```

### Example 5: Meta Skill (Skill Developer)

```json
{
  "skill-developer": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "medium",
    "description": "Create and manage Claude Code skills following Anthropic best practices. Use when creating new skills, modifying skill-rules.json, understanding trigger patterns, working with hooks, debugging skill activation, or implementing progressive disclosure.",
    "promptTriggers": {
      "keywords": [
        "skill",
        "skill-rules.json",
        "trigger",
        "pattern",
        "hook",
        "activation",
        "skill-rules",
        "progressive disclosure",
        "500-line rule",
        "SKILL.md",
        "frontmatter",
        "UserPromptSubmit",
        "PostToolUse"
      ],
      "intentPatterns": [
        "(create|add|build).*?skill",
        "(modify|update|edit).*?skill-rules",
        "(debug|troubleshoot).*?(skill|activation|trigger)",
        "(understand|learn).*?(hook|trigger|pattern)"
      ]
    },
    "fileTriggers": {
      "pathPatterns": [
        ".claude/skills/**/*.md",
        ".claude/skills/skill-rules.json",
        ".claude/hooks/**/*"
      ]
    },
    "skipConditions": {
      "sessionSkillUsed": true
    }
  }
}
```

---

## Validation

### JSON Syntax Validation

Validate JSON syntax before committing:

```bash
jq . .claude/skills/skill-rules.json
```

If valid, it will output the formatted JSON. If invalid, it will show syntax errors.

### Schema Validation Checklist

Before committing skill-rules.json changes:

- [ ] Valid JSON syntax (test with `jq`)
- [ ] `version` field is "1.0"
- [ ] All skills have `type`, `enforcement`, `priority`
- [ ] Skill names use kebab-case
- [ ] Keywords are lowercase strings
- [ ] Intent patterns are valid regex
- [ ] Path patterns are valid glob syntax
- [ ] Content patterns are valid regex with proper escaping
- [ ] All regex special characters properly escaped (`\\`)
- [ ] No duplicate skill names
- [ ] Description under 1024 characters (if present)

### Testing Skill Configuration

Test skill activation after making changes:

```bash
cd .claude/hooks

# Test with keywords
echo '{"prompt":"create a react component"}' | npx tsx skill-activation-prompt.ts

# Test with intent patterns
echo '{"prompt":"I need to add an API endpoint"}' | npx tsx skill-activation-prompt.ts

# Verify expected skills activate
```

---

## Common Errors

### Error 1: Invalid JSON Syntax

**Symptom:**
```
parse error: Expected separator between values at line X
```

**Cause:** Missing comma, trailing comma, or unescaped quotes

**Solution:**
- Use `jq` to validate and identify line number
- Check for missing/extra commas
- Ensure all strings are properly quoted
- Remove trailing commas after last array/object item

### Error 2: Invalid Regex Pattern

**Symptom:** Skill never activates or throws regex error

**Cause:** Invalid regex syntax in `intentPatterns` or `contentPatterns`

**Solution:**
- Test regex patterns in a regex tester
- Escape special characters: `\\.`, `\\(`, `\\[`, etc.
- Use non-greedy matching: `.*?` not `.*`
- Verify regex is valid JavaScript flavor

**Example:**
```json
// WRONG - Missing escape
"contentPatterns": ["z.(object|string)"]

// CORRECT - Escaped dot
"contentPatterns": ["z\\.(object|string)"]
```

### Error 3: Invalid Glob Pattern

**Symptom:** File triggers don't match expected files

**Cause:** Invalid glob syntax in `pathPatterns`

**Solution:**
- Use `**` for recursive matching
- Test glob patterns with actual file paths
- Ensure paths are relative to project root
- Use forward slashes `/` not backslashes `\`

**Example:**
```json
// WRONG - Missing recursive
"pathPatterns": ["apps/frontend/src/*.tsx"]

// CORRECT - Recursive matching
"pathPatterns": ["apps/frontend/src/**/*.tsx"]
```

### Error 4: Skill Not Activating

**Symptom:** Expected skill doesn't activate

**Causes:**
1. Keywords not matching user prompt
2. Intent patterns too specific
3. File paths not matching
4. Content patterns not matching
5. Session skip condition active

**Solutions:**
1. Test with `npx tsx skill-activation-prompt.ts`
2. Add more keywords covering variations
3. Broaden intent patterns
4. Verify file paths match project structure
5. Check session state in `.claude/hooks/state/`
6. Clear session state for testing

### Error 5: Too Many False Positives

**Symptom:** Skill activates too often

**Causes:**
1. Overly broad keywords
2. Greedy intent patterns
3. Content patterns too generic

**Solutions:**
1. Make keywords more specific
2. Add objects to intent patterns
3. Use non-greedy matching (`.*?`)
4. Add more specific content patterns
5. Use path exclusions

---

## Summary

### Required Fields

Every skill MUST have:
- `type`: `"domain"`
- `enforcement`: `"suggest"`
- `priority`: `"high"` | `"medium"` | `"low"`

### Recommended Fields

Every skill SHOULD have:
- `description`: Brief description with keywords
- `promptTriggers`: Keywords and/or intent patterns
- `fileTriggers`: Path patterns and/or content patterns
- `skipConditions`: Session tracking configuration

### Validation Commands

```bash
# Validate JSON syntax
jq . .claude/skills/skill-rules.json

# Test skill activation
echo '{"prompt":"test"}' | npx tsx .claude/hooks/skill-activation-prompt.ts
```

### Best Practices

1. Use consistent skill naming (kebab-case)
2. Include comprehensive keywords
3. Test intent patterns thoroughly
4. Use specific path patterns
5. Escape regex special characters
6. Enable session skip conditions
7. Validate JSON before committing
8. Test with real user prompts

---

**This reference provides complete documentation for configuring skills in skill-rules.json.**

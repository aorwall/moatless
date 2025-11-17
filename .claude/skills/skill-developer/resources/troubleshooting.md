# Troubleshooting Guide

Comprehensive debugging guide for skill activation issues.

---

## Table of Contents

- [Quick Diagnostics](#quick-diagnostics)
- [Skill Not Triggering (UserPromptSubmit)](#skill-not-triggering-userpromptsubmit)
- [False Positives (Too Many Triggers)](#false-positives-too-many-triggers)
- [Hook Not Executing](#hook-not-executing)
- [File Triggers Not Working](#file-triggers-not-working)
- [Session Tracking Issues](#session-tracking-issues)
- [JSON Validation Errors](#json-validation-errors)
- [Testing and Validation](#testing-and-validation)

---

## Quick Diagnostics

### Check List

Run through this checklist first:

1. **Is skill-rules.json valid?**
   ```bash
   jq . .claude/skills/skill-rules.json
   ```

2. **Is the hook executable?**
   ```bash
   ls -l .claude/hooks/*.sh
   # Should show -rwxr-xr-x or similar
   ```

3. **Can hook run manually?**
   ```bash
   cd .claude/hooks
   echo '{"prompt":"test"}' | npx tsx skill-activation-prompt.ts
   ```

4. **Is SKILL.md file present?**
   ```bash
   ls -l .claude/skills/*/SKILL.md
   ```

5. **Is hook registered in settings.json?**
   ```bash
   cat .claude/settings.json | jq .hooks
   ```

If all checks pass, proceed to specific issue sections below.

---

## Skill Not Triggering (UserPromptSubmit)

### Symptom

Expected skill doesn't activate when you mention relevant keywords or intents in your prompt.

### Common Causes

1. **Keywords not matching**
2. **Intent patterns too specific**
3. **Session skip condition active**
4. **SKILL.md file missing**
5. **skill-rules.json has errors**

### Debugging Steps

#### Step 1: Test Hook Manually

```bash
cd .claude/hooks
echo '{"session_id":"test","prompt":"create a new React component"}' | npx tsx skill-activation-prompt.ts
```

**Expected output:**
- Skill reminder message if patterns match
- No output if no patterns match

#### Step 2: Check Keywords

Open `.claude/skills/skill-rules.json` and verify keywords:

```json
{
  "frontend-dev-guidelines": {
    "promptTriggers": {
      "keywords": ["component", "react", "frontend"]
    }
  }
}
```

**Test:**
- Does your prompt contain any of these keywords?
- Are keywords spelled correctly?
- Are they lowercase in the JSON?

#### Step 3: Check Intent Patterns

Verify intent patterns match your prompt structure:

```json
{
  "promptTriggers": {
    "intentPatterns": [
      "(create|add|build).*?(component|page)"
    ]
  }
}
```

**Test:**
- Does your prompt match the pattern?
- Try regex tester: https://regex101.com/ (JavaScript flavor)
- Example: "create a component" matches `(create|add).*?(component)`

#### Step 4: Check Session State

Session tracking might be preventing repeated suggestions:

```bash
# Check session state
ls -l .claude/hooks/state/skills-used-*.json

# View state content
cat .claude/hooks/state/skills-used-*.json
```

If skill is marked as used in session, it won't trigger again. Solutions:
- Start a new session (restart Claude Code)
- Delete state file for testing: `rm .claude/hooks/state/skills-used-*.json`

#### Step 5: Verify SKILL.md Exists

```bash
ls -l .claude/skills/frontend-dev-guidelines/SKILL.md
```

If missing, the skill won't activate even if triggers match.

### Solutions

**Solution 1: Add More Keywords**

```json
{
  "keywords": [
    "component",
    "react",
    "frontend",
    "UI",
    "page",
    "form",
    "modal"
  ]
}
```

Add variations and related terms.

**Solution 2: Broaden Intent Patterns**

```json
{
  "intentPatterns": [
    "(create|add|build|make|implement).*?(component|page|UI|modal)"
  ]
}
```

Add more action verbs and target objects.

**Solution 3: Clear Session State**

```bash
rm .claude/hooks/state/skills-used-*.json
```

For testing only. Session state prevents repeated suggestions.

**Solution 4: Test with Minimal Example**

Create a minimal skill-rules.json entry to isolate the issue:

```json
{
  "test-skill": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "high",
    "promptTriggers": {
      "keywords": ["test"]
    }
  }
}
```

Create `.claude/skills/test-skill/SKILL.md`:

```markdown
---
name: test-skill
description: Test skill
---

# Test Skill

This is a test.
```

Test with:
```bash
echo '{"prompt":"test keyword"}' | npx tsx .claude/hooks/skill-activation-prompt.ts
```

---

## False Positives (Too Many Triggers)

### Symptom

Skill triggers too often, even when not relevant.

### Common Causes

1. **Overly broad keywords**
2. **Greedy intent patterns**
3. **Generic content patterns**
4. **Missing path exclusions**

### Debugging Steps

#### Step 1: Review Keywords

Check for overly generic keywords:

```json
// TOO BROAD
"keywords": ["add", "create", "update", "new"]

// MORE SPECIFIC
"keywords": ["component", "react", "API", "endpoint"]
```

#### Step 2: Check Intent Patterns

Verify intent patterns aren't too greedy:

```json
// TOO BROAD - Matches almost anything
"intentPatterns": ["(create|add|make).*"]

// MORE SPECIFIC - Requires specific object
"intentPatterns": ["(create|add|make).*?(component|service|test)"]
```

#### Step 3: Review Content Patterns

Check for overly generic patterns:

```json
// TOO BROAD - Matches any import
"contentPatterns": ["import"]

// MORE SPECIFIC - Matches specific imports
"contentPatterns": ["import.*react", "from '@tanstack/"]
```

### Solutions

**Solution 1: Make Keywords More Specific**

Replace generic terms with domain-specific ones:

```json
{
  "keywords": [
    "component",    // Instead of "add"
    "endpoint",     // Instead of "create"
    "useQuery",     // Instead of "fetch"
    "@injectable"   // Instead of "service"
  ]
}
```

**Solution 2: Add Objects to Intent Patterns**

Require specific objects after action verbs:

```json
{
  "intentPatterns": [
    "(create|add).*?(component|page|modal)",  // Not just "(create|add)"
    "(implement|add).*?(validation|schema)"   // Not just "(implement|add)"
  ]
}
```

**Solution 3: Add Path Exclusions**

Exclude irrelevant file types:

```json
{
  "fileTriggers": {
    "pathPatterns": ["apps/frontend/src/**/*.tsx"],
    "pathExclusions": [
      "**/*.test.*",
      "**/__tests__/**",
      "**/*.stories.tsx",
      "**/dist/**",
      "**/node_modules/**"
    ]
  }
}
```

**Solution 4: Use More Specific Content Patterns**

Add context to content patterns:

```json
{
  "contentPatterns": [
    "import React from 'react'",      // Not just "react"
    "useQuery|useMutation",           // Specific hooks
    "@injectable\\(\\)",              // Specific decorator
    "from '@tanstack/react-query'"    // Full import path
  ]
}
```

---

## Hook Not Executing

### Symptom

Hook doesn't run at all - no output, no skill activation.

### Common Causes

1. **Hook not executable**
2. **Hook not registered in settings.json**
3. **TypeScript dependencies not installed**
4. **Path issues in settings.json**

### Debugging Steps

#### Step 1: Check Hook Permissions

```bash
ls -l .claude/hooks/skill-activation-prompt.sh
```

Should show: `-rwxr-xr-x` or similar (executable)

If not executable:
```bash
chmod +x .claude/hooks/skill-activation-prompt.sh
chmod +x .claude/hooks/post-tool-use-tracker.sh
```

#### Step 2: Verify settings.json

```bash
cat .claude/settings.json
```

Should contain:
```json
{
  "hooks": {
    "UserPromptSubmit": [{
      "hooks": [{
        "type": "command",
        "command": "$CLAUDE_PROJECT_DIR/.claude/hooks/skill-activation-prompt.sh"
      }]
    }],
    "PostToolUse": [{
      "matcher": "Edit|MultiEdit|Write",
      "hooks": [{
        "type": "command",
        "command": "$CLAUDE_PROJECT_DIR/.claude/hooks/post-tool-use-tracker.sh"
      }]
    }]
  }
}
```

#### Step 3: Test Hook Directly

```bash
# Test shell wrapper
.claude/hooks/skill-activation-prompt.sh <<< '{"prompt":"test"}'

# Test TypeScript hook
cd .claude/hooks
npx tsx skill-activation-prompt.ts <<< '{"prompt":"test"}'
```

If TypeScript hook fails, check dependencies:
```bash
cd .claude/hooks
npm install
```

#### Step 4: Check Hook Output

Add debug output to hook:

```typescript
// In skill-activation-prompt.ts
console.error('DEBUG: Hook started');
console.error('DEBUG: Input:', JSON.stringify(input));
```

Stderr output is visible in Claude Code.

### Solutions

**Solution 1: Make Hooks Executable**

```bash
chmod +x .claude/hooks/*.sh
```

**Solution 2: Register Hooks**

Update `.claude/settings.json`:
```json
{
  "hooks": {
    "UserPromptSubmit": [{
      "hooks": [{"type": "command", "command": "$CLAUDE_PROJECT_DIR/.claude/hooks/skill-activation-prompt.sh"}]
    }]
  }
}
```

**Solution 3: Install Dependencies**

```bash
cd .claude/hooks
npm install
```

**Solution 4: Fix Path Issues**

Use `$CLAUDE_PROJECT_DIR` for absolute path:
```json
"command": "$CLAUDE_PROJECT_DIR/.claude/hooks/skill-activation-prompt.sh"
```

---

## File Triggers Not Working

### Symptom

Skill doesn't activate when editing files that should match path/content patterns.

### Common Causes

1. **Glob patterns don't match file paths**
2. **Content patterns don't match file contents**
3. **Path exclusions blocking matches**
4. **PostToolUse hook not running**

### Debugging Steps

#### Step 1: Test Path Pattern Manually

```bash
# Test if file path matches pattern
# Use a glob tester or test manually

# Example: Does this match apps/frontend/src/**/*.tsx?
echo "apps/frontend/src/features/tasks/task-list.tsx"
# YES - matches

echo "apps/frontend/src/features/tasks/task-list.test.tsx"
# Depends on exclusions
```

#### Step 2: Check File Path

Verify the actual file path:

```bash
pwd
# Should be in project root

# File paths are relative to project root
ls apps/frontend/src/features/tasks/task-list.tsx
```

#### Step 3: Check Content Patterns

Verify file actually contains the pattern:

```bash
# Search for pattern in file
grep "import.*react" apps/frontend/src/features/tasks/task-list.tsx

# Or use content pattern directly
grep -E "import.*react" apps/frontend/src/features/tasks/task-list.tsx
```

#### Step 4: Check Path Exclusions

Verify file isn't excluded:

```json
{
  "pathExclusions": [
    "**/*.test.*",    // Excludes *.test.tsx
    "**/__tests__/**" // Excludes files in __tests__/
  ]
}
```

Test:
```bash
# Does file match exclusion?
echo "task-list.test.tsx" | grep -E "\.test\."
# YES - excluded
```

### Solutions

**Solution 1: Fix Path Patterns**

Use correct glob syntax:

```json
{
  "pathPatterns": [
    "apps/frontend/src/**/*.tsx",  // Recursive
    "apps/backend/src/**/*.ts"     // Not: apps/backend/src/*.ts
  ]
}
```

**Solution 2: Verify Content Patterns**

Test regex against file content:

```bash
# Extract pattern
PATTERN="import.*react"

# Test against file
grep -E "$PATTERN" your-file.tsx
```

**Solution 3: Review Exclusions**

Remove or adjust exclusions if too broad:

```json
{
  "pathExclusions": [
    "**/*.test.*",           // Keep
    "**/__tests__/**"        // Keep
    // Remove if blocking legitimate files
  ]
}
```

**Solution 4: Use Simpler Patterns**

Start with basic patterns and add complexity:

```json
{
  "pathPatterns": [
    "apps/frontend/**/*.tsx"  // Simple, broad
  ]
}
```

Then narrow down based on testing.

---

## Session Tracking Issues

### Symptom

Skill triggers once but never again, even in new conversations.

### Cause

Session state not clearing properly or session ID persisting.

### Solutions

**Solution 1: Clear Session State**

```bash
# Remove all session state
rm -rf .claude/hooks/state/

# Or remove specific session
rm .claude/hooks/state/skills-used-{session-id}.json
```

**Solution 2: Disable Session Tracking (Testing)**

Temporarily disable for testing:

```json
{
  "skipConditions": {
    "sessionSkillUsed": false  // Disable session tracking
  }
}
```

**Note:** Re-enable after testing to prevent repeated suggestions.

**Solution 3: Check Session ID**

Session ID should change between conversations:

```bash
# Check current session files
ls -lt .claude/hooks/state/skills-used-*.json | head -5
```

If same session ID persists, restart Claude Code.

---

## JSON Validation Errors

### Symptom

skill-rules.json won't parse or throws syntax errors.

### Common Errors

#### Error 1: Trailing Comma

```json
{
  "keywords": ["test",]  // ERROR: Trailing comma
}
```

**Fix:**
```json
{
  "keywords": ["test"]  // CORRECT
}
```

#### Error 2: Missing Comma

```json
{
  "type": "domain"
  "enforcement": "suggest"  // ERROR: Missing comma
}
```

**Fix:**
```json
{
  "type": "domain",
  "enforcement": "suggest"  // CORRECT
}
```

#### Error 3: Unescaped Special Characters

```json
{
  "contentPatterns": ["z.(object|string)"]  // ERROR: Unescaped dot
}
```

**Fix:**
```json
{
  "contentPatterns": ["z\\.(object|string)"]  // CORRECT
}
```

### Solutions

**Solution 1: Use jq to Validate**

```bash
jq . .claude/skills/skill-rules.json
```

If valid, outputs formatted JSON. If invalid, shows line number and error.

**Solution 2: Use JSON Linter**

Copy JSON to: https://jsonlint.com/

**Solution 3: Format with jq**

```bash
# Format and save back
jq . .claude/skills/skill-rules.json > /tmp/formatted.json
mv /tmp/formatted.json .claude/skills/skill-rules.json
```

---

## Testing and Validation

### Manual Testing Workflow

1. **Make changes to skill-rules.json**
   ```bash
   vim .claude/skills/skill-rules.json
   ```

2. **Validate JSON syntax**
   ```bash
   jq . .claude/skills/skill-rules.json
   ```

3. **Test prompt triggers**
   ```bash
   cd .claude/hooks
   echo '{"prompt":"create a react component"}' | npx tsx skill-activation-prompt.ts
   ```

4. **Verify expected output**
   - Should see skill reminder if triggers match
   - No output if no triggers match

5. **Test with variations**
   ```bash
   echo '{"prompt":"add a new component"}' | npx tsx skill-activation-prompt.ts
   echo '{"prompt":"build a modal dialog"}' | npx tsx skill-activation-prompt.ts
   ```

6. **Clear session state for retesting**
   ```bash
   rm .claude/hooks/state/skills-used-*.json
   ```

### Automated Testing Script

Create a test script:

```bash
#!/bin/bash
# test-skills.sh

cd .claude/hooks

# Test cases
TESTS=(
  '{"prompt":"create a react component"}'
  '{"prompt":"add an API endpoint"}'
  '{"prompt":"write unit tests"}'
  '{"prompt":"unrelated prompt"}'
)

for test in "${TESTS[@]}"; do
  echo "Testing: $test"
  echo "$test" | npx tsx skill-activation-prompt.ts
  echo "---"
done
```

Run:
```bash
chmod +x test-skills.sh
./test-skills.sh
```

### Validation Checklist

- [ ] JSON syntax valid (`jq` passes)
- [ ] All skills have required fields
- [ ] Keywords are lowercase strings
- [ ] Intent patterns are valid regex
- [ ] Path patterns are valid glob
- [ ] Content patterns escape special chars
- [ ] Hooks are executable
- [ ] Hooks registered in settings.json
- [ ] TypeScript dependencies installed
- [ ] Manual hook test works
- [ ] Prompt triggers tested with real prompts
- [ ] File triggers tested with real files
- [ ] Session tracking works as expected

---

## Summary

### Quick Fixes

| Problem | Quick Fix |
|---------|-----------|
| Skill not triggering | Add more keywords, test with `npx tsx` |
| Too many triggers | Make keywords more specific |
| Hook not running | Check permissions: `chmod +x .claude/hooks/*.sh` |
| File triggers broken | Verify path patterns match actual paths |
| Session stuck | Clear state: `rm .claude/hooks/state/*` |
| JSON errors | Validate: `jq . skill-rules.json` |

### Testing Commands

```bash
# Validate JSON
jq . .claude/skills/skill-rules.json

# Test hook manually
cd .claude/hooks
echo '{"prompt":"test"}' | npx tsx skill-activation-prompt.ts

# Check hook permissions
ls -l .claude/hooks/*.sh

# Clear session state
rm .claude/hooks/state/skills-used-*.json
```

### Best Practices

1. **Always validate JSON** before committing
2. **Test triggers manually** with real prompts
3. **Start simple** then add complexity
4. **Clear session state** when testing
5. **Check hook output** for errors
6. **Use specific patterns** to avoid false positives
7. **Document patterns** with comments in JSON

---

**This troubleshooting guide covers all common skill activation issues and their solutions.**

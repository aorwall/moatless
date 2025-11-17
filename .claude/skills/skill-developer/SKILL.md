---
name: skill-developer
description: Create and manage Claude Code skills following Anthropic best practices. Use when creating new skills, modifying skill-rules.json, understanding trigger patterns, working with hooks, debugging skill activation, or implementing progressive disclosure. Covers skill structure, YAML frontmatter, trigger types (keywords, intent patterns, file paths, content patterns), enforcement levels (suggest), hook mechanisms (UserPromptSubmit, PostToolUse), and the 500-line rule.
---

# Skill Developer Guide

## Purpose

Comprehensive guide for creating and managing skills in Claude Code with auto-activation system, following Anthropic's official best practices including the 500-line rule and progressive disclosure pattern.

## When to Use This Skill

Automatically activates when you mention:
- Creating or adding skills
- Modifying skill triggers or rules
- Understanding how skill activation works
- Debugging skill activation issues
- Working with skill-rules.json
- Hook system mechanics
- Claude Code best practices
- Progressive disclosure
- YAML frontmatter
- 500-line rule
- Writing skill resources

---

## System Overview

### Two-Hook Architecture

**1. UserPromptSubmit Hook** (Proactive Suggestions)
- **File**: `.claude/hooks/skill-activation-prompt.ts`
- **Trigger**: BEFORE Claude sees user's prompt
- **Purpose**: Suggest relevant skills based on keywords + intent patterns
- **Method**: Injects formatted reminder as context (stdout → Claude's input)
- **Use Cases**: Topic-based skills, implicit work detection

**2. PostToolUse Hook** (File Tracking)
- **File**: `.claude/hooks/post-tool-use-tracker.sh`
- **Trigger**: AFTER Edit/Write tool execution
- **Purpose**: Track file changes and auto-detect project structure
- **Method**: Analyzes edited files, updates tracking state
- **Use Cases**: Contextual skill suggestions based on files modified

### Configuration File

**Location**: `.claude/skills/skill-rules.json`

Defines:
- All skills and their trigger conditions
- Enforcement levels (suggest is standard for domain skills)
- File path patterns (glob)
- Content detection patterns (regex)
- Skip conditions (session tracking, file markers)

---

## Skill Types

This project uses **domain skills** exclusively - comprehensive guidance for specific development areas.

### Domain Skills

**Purpose:** Provide comprehensive guidance for specific areas

**Characteristics:**
- Type: `"domain"`
- Enforcement: `"suggest"`
- Priority: `"high"` or `"medium"`
- Advisory, not mandatory
- Topic or domain-specific
- Comprehensive documentation

**Examples in This Project:**
- `frontend-dev-guidelines` - React 19 + TanStack Query/Router patterns
- `backend-dev-guidelines` - Hono + InversifyJS + Drizzle patterns
- `testing-guidelines` - Vitest testing patterns with tier classification
- `e2e-testing` - End-to-end testing patterns
- `skill-developer` - Skill creation and management

**When to Create a Domain Skill:**
- Complex systems requiring deep knowledge
- Best practices documentation
- Architectural patterns
- How-to guides
- Feature-specific development patterns

---

## Quick Start: Creating a New Skill

### Step 1: Create Skill File

**Location:** `.claude/skills/{skill-name}/SKILL.md`

**Template:**
```markdown
---
name: my-new-skill
description: Brief description including keywords that trigger this skill. Mention topics, file types, and use cases. Be explicit about trigger terms.
---

# My New Skill

## Purpose
What this skill helps with

## When to Use This Skill
Specific scenarios and conditions

## Quick Reference

| Need to... | See |
|------------|-----|
| Topic 1 | [Resource Name](resources/resource-file.md) |
| Topic 2 | [Resource Name](resources/resource-file.md) |

## Core Principles

### 1. Principle Name
Brief description

### 2. Another Principle
Brief description

## Critical Anti-Patterns to Avoid

### Pattern Category
❌ **NEVER** do this
```language
// WRONG example
```

✅ **CORRECT** approach
```language
// CORRECT example
```

## Navigation Guide

### Essential Resources (Read First)
1. **[Resource Name](resources/resource-file.md)** - Description

### Implementation Guides
2. **[Resource Name](resources/resource-file.md)** - Description

## Quick Start Checklist

### Before Starting Task
- [ ] Checklist item 1
- [ ] Checklist item 2

## Getting Help

### When Working On...
- **Topic**: See [Resource Name](resources/resource-file.md)
```

**Best Practices:**
- ✅ **Name**: Lowercase, hyphens, gerund form (verb + -ing) preferred
- ✅ **Description**: Include ALL trigger keywords/phrases (max 1024 chars)
- ✅ **Content**: Under 500 lines - use reference files for details
- ✅ **Examples**: Real code examples from this project
- ✅ **Structure**: Clear headings, lists, code blocks
- ✅ **Navigation**: Quick Reference table linking to resources
- ✅ **No External Links**: Self-contained, no links to docs/patterns/

### Step 2: Create Resource Files

**Location:** `.claude/skills/{skill-name}/resources/`

**Resource File Guidelines:**
- Store detailed content in resources/ directory
- Each resource covers a specific topic (e.g., component-patterns.md, api-contract-pattern.md)
- Resources can be longer than 500 lines
- Link to resources from main SKILL.md
- Keep resources self-contained (no external pattern links)

**Example Structure:**
```
.claude/skills/my-new-skill/
├── SKILL.md                    # <500 lines
└── resources/
    ├── core-concepts.md        # Detailed topic 1
    ├── implementation.md       # Detailed topic 2
    └── complete-examples.md    # Full examples
```

### Step 3: Add to skill-rules.json

See [Skill Rules Reference](resources/skill-rules-reference.md) for complete schema.

**Basic Template:**
```json
{
  "my-new-skill": {
    "type": "domain",
    "enforcement": "suggest",
    "priority": "medium",
    "description": "Brief description of what this skill covers",
    "promptTriggers": {
      "keywords": ["keyword1", "keyword2"],
      "intentPatterns": ["(create|add).*?something"]
    },
    "fileTriggers": {
      "pathPatterns": ["path/to/files/**/*.ts"],
      "pathExclusions": ["**/*.test.*"]
    },
    "skipConditions": {
      "sessionSkillUsed": true
    }
  }
}
```

### Step 4: Test Triggers

**Test UserPromptSubmit:**
```bash
cd .claude/hooks
echo '{"session_id":"test","prompt":"your test prompt"}' | \
  npx tsx skill-activation-prompt.ts
```

Expected output: Skill reminder if keywords/patterns match

**Test with Real Prompts:**
- Test with keywords: "create component", "add API endpoint"
- Test with intent patterns: "implement validation"
- Verify skill activates appropriately

### Step 5: Follow Anthropic Best Practices

✅ Keep SKILL.md under 500 lines
✅ Use progressive disclosure with reference files
✅ Add table of contents to reference files > 100 lines
✅ Write detailed description with trigger keywords
✅ Test with 3+ real scenarios before documenting
✅ Iterate based on actual usage
✅ Remove all links to docs/patterns/ (self-contained)
✅ Use Quick Reference table for navigation

---

## Enforcement Levels

This project uses **suggest** enforcement for all domain skills.

### SUGGEST (Recommended - Our Standard)

- Reminder injected before Claude sees prompt
- Claude is aware of relevant skills
- Not enforced, just advisory
- **Use For**: Domain guidance, best practices, how-to guides

**Example:** Frontend development guidelines, backend patterns, testing patterns

### Why We Don't Use BLOCK

- Blocking enforcement can interrupt workflow
- Suggest mode provides guidance without friction
- Domain skills are advisory, not mandatory
- Session tracking prevents repeated suggestions

---

## Trigger Configuration

### Prompt Triggers

**Keywords:**
- Explicit terms that trigger the skill
- Case-insensitive matching
- Examples: "component", "API", "test", "validation"

**Intent Patterns:**
- Regex patterns matching user intent
- Capture implicit work detection
- Examples: `(create|add).*?(component|page)`, `(test|mock).*?service`

### File Triggers

**Path Patterns:**
- Glob patterns matching file paths
- Examples: `apps/frontend/src/**/*.tsx`, `apps/backend/src/**/*.ts`
- Use `**` for recursive matching

**Path Exclusions:**
- Files to exclude from triggering
- Examples: `**/*.test.*`, `**/__tests__/**`

**Content Patterns:**
- Regex patterns matching file content
- Examples: `import.*react`, `@injectable|@inject`
- Trigger when specific imports or patterns detected

See [Trigger Patterns Reference](resources/trigger-patterns.md) for detailed examples.

---

## Skip Conditions & Session Tracking

### Session Tracking

**Purpose:** Don't suggest the same skill repeatedly in one session

**How it works:**
- First mention → Skill suggested
- Same session → Skill not suggested again
- Different session → Skill suggested again

**Configuration:**
```json
"skipConditions": {
  "sessionSkillUsed": true
}
```

**State File:** `.claude/hooks/state/skills-used-{session_id}.json`

---

## Writing Skill Resources

For comprehensive guidance on writing skill resource files, see:

### [Writing Skill Resources](resources/writing-skill-resources.md)
Complete guide to creating pattern documents for AI agents:
- Document structure and section hierarchy
- Imperative directives (MUST, SHOULD, MAY, NEVER)
- Code example strategy (when to include, when to omit)
- Correct/Wrong pattern examples
- Avoiding AI confusion
- Cross-reference strategy
- Executable elements (commands, file paths)
- Validation patterns and checklists

This resource is adapted from the project's pattern writing guidelines and provides essential information for creating high-quality skill documentation.

---

## Testing Checklist

When creating a new skill, verify:

- [ ] Skill file created in `.claude/skills/{name}/SKILL.md`
- [ ] Proper frontmatter with name and description
- [ ] Entry added to `skill-rules.json`
- [ ] Keywords tested with real prompts
- [ ] Intent patterns tested with variations
- [ ] File path patterns tested with actual files
- [ ] Content patterns tested against file contents
- [ ] Priority level matches importance
- [ ] No false positives in testing
- [ ] No false negatives in testing
- [ ] JSON syntax validated: `jq . .claude/skills/skill-rules.json`
- [ ] **SKILL.md under 500 lines** ⭐
- [ ] Reference files created for detailed content
- [ ] Quick Reference table includes all resources
- [ ] No external links to docs/patterns/
- [ ] All code examples are self-contained

---

## Resource Files

For detailed information on specific topics, see:

### [Writing Skill Resources](resources/writing-skill-resources.md)
Complete guide to creating pattern documents for AI agents:
- Document structure for AI consumption
- Imperative directives and rule specification
- Strategic code example usage
- Avoiding AI confusion with specificity
- Cross-references within skill resources

### [Trigger Patterns Reference](resources/trigger-patterns.md)
Complete guide to all trigger types:
- Keyword triggers (explicit topic matching)
- Intent patterns (implicit action detection)
- File path triggers (glob patterns)
- Content patterns (regex in files)
- Real examples from this project's skills

### [Skill Rules Reference](resources/skill-rules-reference.md)
Complete skill-rules.json schema:
- Full JSON schema documentation
- Field-by-field explanations
- Complete domain skill example
- Validation guide and common errors

### [Troubleshooting Guide](resources/troubleshooting.md)
Comprehensive debugging guide:
- Skill not triggering (UserPromptSubmit)
- False positives (too many triggers)
- Hook not executing at all
- Testing and validation approaches

---

## Quick Reference Summary

### Create New Skill (5 Steps)

1. Create `.claude/skills/{name}/SKILL.md` with frontmatter (<500 lines)
2. Create resource files in `resources/` directory
3. Add entry to `.claude/skills/skill-rules.json`
4. Test with `npx tsx` commands
5. Refine patterns based on testing

### Trigger Types

- **Keywords**: Explicit topic mentions
- **Intent**: Implicit action detection
- **File Paths**: Location-based activation
- **Content**: Technology-specific detection

See [Trigger Patterns Reference](resources/trigger-patterns.md) for details.

### Enforcement

- **SUGGEST**: Advisory mode (standard for domain skills)
- Session tracking prevents repeated suggestions

### Skip Conditions

- **Session tracking**: Automatic (prevents repeated suggestions in same session)

### Anthropic Best Practices

✅ **500-line rule**: Keep SKILL.md under 500 lines
✅ **Progressive disclosure**: Use resource files for details
✅ **Quick Reference**: Navigation table at top of SKILL.md
✅ **Self-contained**: No external links to docs/patterns/
✅ **Rich descriptions**: Include all trigger keywords (max 1024 chars)
✅ **Test first**: Build 3+ evaluations before extensive documentation
✅ **Gerund naming**: Prefer verb + -ing (e.g., "processing-pdfs")

### Troubleshoot

Test hooks manually:
```bash
# UserPromptSubmit
cd .claude/hooks
echo '{"prompt":"test component creation"}' | npx tsx skill-activation-prompt.ts

# Check skill-rules.json syntax
jq . .claude/skills/skill-rules.json
```

See [Troubleshooting Guide](resources/troubleshooting.md) for complete debugging.

---

## Related Files

**Configuration:**
- `.claude/skills/skill-rules.json` - Master configuration
- `.claude/hooks/state/` - Session tracking
- `.claude/settings.json` - Hook registration

**Hooks:**
- `.claude/hooks/skill-activation-prompt.ts` - UserPromptSubmit
- `.claude/hooks/post-tool-use-tracker.sh` - PostToolUse tracking

**All Skills:**
- `.claude/skills/*/SKILL.md` - Skill content files
- `.claude/skills/*/resources/` - Detailed resource files

---

## Project-Specific Examples

**Existing Skills in This Project:**
- `frontend-dev-guidelines`: 13 resources covering React 19, TanStack Query/Router, Tailwind v4
- `backend-dev-guidelines`: 13 resources covering Hono, InversifyJS, Drizzle ORM
- `testing-guidelines`: 4 resources covering Vitest, tier classification, strategic mocking
- `e2e-testing`: End-to-end testing patterns
- `skill-developer`: This skill for creating and managing skills

Each skill follows the pattern: SKILL.md (<500 lines) + resources/ directory with detailed content.

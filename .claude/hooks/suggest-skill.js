#!/usr/bin/env node

/**
 * Skill Suggestion Hook
 *
 * Automatically suggests relevant skills based on:
 * - Keywords and intent patterns in user prompts (UserPromptSubmit hook)
 * - File paths being accessed/edited (PreToolUse hook for Read/Edit/Write tools)
 *
 * Skills are only suggested once per session (tracked via session state).
 */

const fs = require('fs');
const path = require('path');
const os = require('os');

// Configuration
const SESSION_STATE_DIR = path.join(os.tmpdir(), 'claude-skill-sessions');
const SESSION_CLEANUP_AGE_MS = 24 * 60 * 60 * 1000; // 24 hours

/**
 * Load skill rules from skill-rules.json
 */
function loadRules() {
  const projectDir = process.env.CLAUDE_PROJECT_DIR;
  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG loadRules: projectDir=' + projectDir);
  }
  if (!projectDir) {
    return null;
  }

  const rulesPath = path.join(projectDir, '.claude', 'skills', 'skill-rules.json');
  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG loadRules: rulesPath=' + rulesPath);
    console.error('DEBUG loadRules: exists=' + fs.existsSync(rulesPath));
  }
  try {
    const content = fs.readFileSync(rulesPath, 'utf-8');
    return JSON.parse(content);
  } catch (e) {
    if (process.env.DEBUG_SKILL_HOOK) {
      console.error('DEBUG loadRules: error=' + e.message);
    }
    return null;
  }
}

/**
 * Ensure session state directory exists
 */
function ensureSessionDir() {
  if (!fs.existsSync(SESSION_STATE_DIR)) {
    fs.mkdirSync(SESSION_STATE_DIR, { recursive: true });
  }
}

/**
 * Get session state file path
 */
function getSessionStatePath(sessionId) {
  return path.join(SESSION_STATE_DIR, `${sessionId}.json`);
}

/**
 * Load session state, creating if doesn't exist
 */
function getSessionState(sessionId) {
  ensureSessionDir();
  const statePath = getSessionStatePath(sessionId);

  try {
    if (fs.existsSync(statePath)) {
      const content = fs.readFileSync(statePath, 'utf-8');
      return JSON.parse(content);
    }
  } catch (e) {
    // Ignore errors, return fresh state
  }

  return {
    suggestedSkills: [],
    lastUpdated: new Date().toISOString()
  };
}

/**
 * Save session state
 */
function saveSessionState(sessionId, state) {
  ensureSessionDir();
  const statePath = getSessionStatePath(sessionId);
  state.lastUpdated = new Date().toISOString();
  fs.writeFileSync(statePath, JSON.stringify(state, null, 2));
}

/**
 * Clean up old session files (older than 24h)
 */
function cleanupOldSessions() {
  try {
    if (!fs.existsSync(SESSION_STATE_DIR)) return;

    const files = fs.readdirSync(SESSION_STATE_DIR);
    const now = Date.now();

    for (const file of files) {
      const filePath = path.join(SESSION_STATE_DIR, file);
      const stat = fs.statSync(filePath);

      if (now - stat.mtimeMs > SESSION_CLEANUP_AGE_MS) {
        fs.unlinkSync(filePath);
      }
    }
  } catch (e) {
    // Ignore cleanup errors
  }
}

/**
 * Check if a keyword matches in the prompt (case-insensitive, word boundary)
 */
function matchKeyword(prompt, keyword) {
  const escapedKeyword = keyword.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const regex = new RegExp(`\\b${escapedKeyword}\\b`, 'i');
  return regex.test(prompt);
}

/**
 * Match prompt against skill's promptTriggers
 */
function matchPromptTriggers(prompt, skill) {
  const triggers = skill.promptTriggers;
  if (!triggers) return false;

  // Check keywords
  if (triggers.keywords) {
    for (const keyword of triggers.keywords) {
      if (matchKeyword(prompt, keyword)) {
        return true;
      }
    }
  }

  // Check intent patterns
  if (triggers.intentPatterns) {
    for (const pattern of triggers.intentPatterns) {
      try {
        const regex = new RegExp(pattern, 'i');
        if (regex.test(prompt)) {
          return true;
        }
      } catch (e) {
        // Invalid regex, skip
      }
    }
  }

  return false;
}

/**
 * Convert glob pattern to regex
 */
function globToRegex(pattern) {
  // First escape special regex characters (except * and ?)
  let regex = pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&');

  // Then convert glob patterns
  regex = regex
    .replace(/\*\*/g, '<<<GLOBSTAR>>>')
    .replace(/\*/g, '[^/]*')
    .replace(/<<<GLOBSTAR>>>/g, '.*')
    .replace(/\?/g, '.');

  return new RegExp(`^${regex}$`);
}

/**
 * Match file path against skill's fileTriggers
 */
function matchFileTriggers(filePath, skill) {
  const triggers = skill.fileTriggers;
  if (!triggers || !triggers.pathPatterns) return false;

  // Normalize path (remove leading project dir if present)
  const projectDir = process.env.CLAUDE_PROJECT_DIR || '';
  let normalizedPath = filePath;
  if (projectDir && filePath.startsWith(projectDir)) {
    normalizedPath = filePath.slice(projectDir.length).replace(/^\//, '');
  }

  // Debug output
  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG matchFileTriggers: filePath=' + filePath + ', normalizedPath=' + normalizedPath);
  }

  // Check exclusions first
  if (triggers.pathExclusions) {
    for (const exclusion of triggers.pathExclusions) {
      try {
        const regex = globToRegex(exclusion);
        if (regex.test(normalizedPath)) {
          if (process.env.DEBUG_SKILL_HOOK) {
            console.error('DEBUG: excluded by ' + exclusion);
          }
          return false;
        }
      } catch (e) {
        // Invalid pattern, skip
      }
    }
  }

  // Check path patterns
  for (const pattern of triggers.pathPatterns) {
    try {
      const regex = globToRegex(pattern);
      const matches = regex.test(normalizedPath);
      if (process.env.DEBUG_SKILL_HOOK) {
        console.error('DEBUG: pattern=' + pattern + ', regex=' + regex + ', matches=' + matches);
      }
      if (matches) {
        return true;
      }
    } catch (e) {
      // Invalid pattern, skip
    }
  }

  return false;
}

/**
 * Format suggestion message for Claude
 */
function formatSuggestion(skills, rulesData) {
  if (skills.length === 0) return null;

  const lines = ['<skill-suggestion>'];

  if (skills.length === 1) {
    const skillName = skills[0];
    const skillData = rulesData.skills[skillName];
    lines.push(`Based on your request, consider using the **${skillName}** skill.`);
    if (skillData && skillData.description) {
      lines.push(`This skill provides: ${skillData.description}`);
    }
    lines.push(`To activate: Use the Skill tool with skill="${skillName}"`);
  } else {
    lines.push('Based on your request, the following skills may be helpful:');
    for (const skillName of skills) {
      const skillData = rulesData.skills[skillName];
      const desc = skillData && skillData.description ? ` - ${skillData.description}` : '';
      lines.push(`- **${skillName}**${desc}`);
    }
    lines.push('To activate a skill: Use the Skill tool with skill="<skill-name>"');
  }

  lines.push('</skill-suggestion>');
  return lines.join('\n');
}

/**
 * Find matching skills for a prompt
 */
function findPromptMatches(prompt, rulesData, suggestedSkills) {
  const matches = [];

  for (const [skillName, skill] of Object.entries(rulesData.skills)) {
    // Skip if already suggested in this session
    if (skill.skipConditions?.sessionSkillUsed && suggestedSkills.includes(skillName)) {
      continue;
    }

    if (matchPromptTriggers(prompt, skill)) {
      matches.push(skillName);
    }
  }

  return matches;
}

/**
 * Find matching skills for a file path
 */
function findFileMatches(filePath, rulesData, suggestedSkills) {
  const matches = [];

  for (const [skillName, skill] of Object.entries(rulesData.skills)) {
    // Skip if already suggested in this session
    if (skill.skipConditions?.sessionSkillUsed && suggestedSkills.includes(skillName)) {
      continue;
    }

    if (matchFileTriggers(filePath, skill)) {
      matches.push(skillName);
    }
  }

  return matches;
}

/**
 * Main hook handler
 */
async function main() {
  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG: main() started');
  }

  // Read input from stdin
  let input = '';
  for await (const chunk of process.stdin) {
    input += chunk;
  }

  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG: input read, length=' + input.length);
    console.error('DEBUG: input=' + input.substring(0, 200));
  }

  let hookData;
  try {
    hookData = JSON.parse(input);
  } catch (e) {
    // Invalid input, exit silently
    if (process.env.DEBUG_SKILL_HOOK) {
      console.error('DEBUG: failed to parse JSON:', e.message);
    }
    process.exit(0);
  }

  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG: parsed hookData');
  }

  // Load rules
  const rulesData = loadRules();
  if (!rulesData || !rulesData.skills) {
    if (process.env.DEBUG_SKILL_HOOK) {
      console.error('DEBUG: failed to load rules or no skills');
    }
    process.exit(0);
  }

  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG: loaded ' + Object.keys(rulesData.skills).length + ' skills');
  }

  // Get session ID
  const sessionId = hookData.session_id || 'default';

  // Load session state
  const sessionState = getSessionState(sessionId);

  // Clean up old sessions occasionally (1% chance)
  if (Math.random() < 0.01) {
    cleanupOldSessions();
  }

  let matches = [];
  const hookName = hookData.hook_event_name || '';

  if (hookName === 'UserPromptSubmit') {
    // Match against prompt content
    const prompt = hookData.prompt || '';
    matches = findPromptMatches(prompt, rulesData, sessionState.suggestedSkills);
  } else if (hookName === 'PreToolUse') {
    // Match against file path for Read/Edit/Write tools
    const toolName = hookData.tool_name || '';
    if (['Read', 'Edit', 'Write', 'MultiEdit'].includes(toolName)) {
      const toolInput = hookData.tool_input || {};
      const filePath = toolInput.file_path || toolInput.path || '';
      if (filePath) {
        matches = findFileMatches(filePath, rulesData, sessionState.suggestedSkills);
      }
    }
  }

  // Debug: log to stderr
  if (process.env.DEBUG_SKILL_HOOK) {
    console.error('DEBUG: hookName=' + hookName + ', matches=' + JSON.stringify(matches));
  }

  // If we have matches, output suggestion and update session state
  if (matches.length > 0) {
    // Update session state with newly suggested skills
    for (const skillName of matches) {
      if (!sessionState.suggestedSkills.includes(skillName)) {
        sessionState.suggestedSkills.push(skillName);
      }
    }
    saveSessionState(sessionId, sessionState);

    // Format and output suggestion
    const suggestion = formatSuggestion(matches, rulesData);
    if (suggestion) {
      const output = {
        hookSpecificOutput: {
          hookEventName: hookName,
          additionalContext: suggestion
        }
      };
      console.log(JSON.stringify(output));
    }
  }
}

main().catch(() => process.exit(0));

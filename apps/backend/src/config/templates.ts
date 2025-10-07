import { RepositoryTemplate } from '@moatless/types'
import { MOATLESS_SYSTEM_PROMPT } from './system-prompts.js'

/**
 * Available repository templates
 * These templates are cloned fresh each time a new vibe task is created
 */
export const TEMPLATES: RepositoryTemplate[] = [
  new RepositoryTemplate(
    'hello-world-ui',
    'Basic Hello Website',
    'https://github.com/aorwall/hello-world-ui.git',
    'bun install',
    {
      serverStartCommands: ['bun run dev'],
      buildCommand: 'bun run build',
      takeScreenshots: true,
      enableInspector: true,
    },
    MOATLESS_SYSTEM_PROMPT,
  ),
]

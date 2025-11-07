import { RepositoryTemplate } from '@moatless/types'
import {
  COMPONENT_PROTOTYPE_SYSTEM_PROMPT,
  FIGMA_PROTOTYPE_SYSTEM_PROMPT,
  MOATLESS_SYSTEM_PROMPT,
  REVEALJS_SYSTEM_PROMPT,
} from './system-prompts'

/**
 * Available repository templates
 * These templates are cloned fresh each time a new vibe task is created
 */
export const TEMPLATES: RepositoryTemplate[] = [
  new RepositoryTemplate(
    'figma-prototype',
    'Figma Prototype',
    'https://github.com/aorwall/hello-world-ui.git',
    'bun install',
    {
      serverStartCommands: ['bun run preview'],
      buildCommand: 'bun run build',
      takeScreenshots: true,
      enableInspector: true,
    },
    FIGMA_PROTOTYPE_SYSTEM_PROMPT,
  ),
  new RepositoryTemplate(
    'component-prototype',
    'Component Prototype',
    'https://github.com/aorwall/hello-world-ui.git',
    'bun install',
    {
      serverStartCommands: ['bun run preview'],
      buildCommand: 'bun run build',
      takeScreenshots: true,
      enableInspector: true,
    },
    COMPONENT_PROTOTYPE_SYSTEM_PROMPT,
  ),
  new RepositoryTemplate(
    'hello-world-ui',
    'Basic Hello Website',
    'https://github.com/aorwall/hello-world-ui.git',
    'bun install --ignore-scripts',
    {
      serverStartCommands: ['bun run preview'],
      buildCommand: 'bun run build',
      takeScreenshots: true,
      enableInspector: true,
    },
    MOATLESS_SYSTEM_PROMPT,
  ),
  new RepositoryTemplate(
    'revealjs-presentation',
    'Reveal.js Presentation',
    'https://github.com/hakimel/reveal.js.git',
    'npm install',
    {
      serverStartCommands: ['npm start -- --port {PORT}'],
      buildCommand: 'npm run build',
      takeScreenshots: true,
      enableInspector: false,
    },
    REVEALJS_SYSTEM_PROMPT,
  ),
]

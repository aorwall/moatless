import type { LucideIcon } from 'lucide-react'
import { icons } from 'lucide-react'

// icons is a namespace where every property is a LucideIcon — cast to a
// Record so we can do safe dynamic lookup by name (keys are PascalCase).
const iconsMap = icons as Record<string, LucideIcon>

/**
 * Look up a Lucide icon component by its PascalCase name (e.g. "GitCommit").
 * Returns undefined when the name is not a known icon.
 */
export function getLucideIcon(name: string): LucideIcon | undefined {
  return iconsMap[name]
}

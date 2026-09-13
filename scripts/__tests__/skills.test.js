import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { describe, expect, it } from 'vitest'

const root = path.resolve(import.meta.dirname, '../..')
const skills = path.join(root, '.agents/skills')
// Use the frontend's declared YAML dependency, which also parses skill content.
const { load } = createRequire(path.join(root, 'apps/frontend/package.json'))('js-yaml')
const directories = readdirSync(skills, { withFileTypes: true }).filter((entry) =>
  entry.isDirectory(),
)
function filesIn(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filename = path.join(directory, entry.name)
    return entry.isDirectory() ? filesIn(filename) : [filename]
  })
}

describe('repository skill metadata and references', () => {
  it('contains skills with unique names', () => {
    expect(directories.length).toBeGreaterThan(0)
    const names = directories.map((entry) => {
      const source = readFileSync(path.join(skills, entry.name, 'SKILL.md'), 'utf8')
      const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n/.exec(source)
      expect(match, `${entry.name}: missing frontmatter`).not.toBeNull()
      const metadata = load(match[1])
      expect(metadata.name).toBe(entry.name)
      expect(metadata.name).toMatch(/^[a-z0-9]+(?:-[a-z0-9]+)*$/)
      expect(metadata.description).toBeTypeOf('string')
      expect(metadata.description.trim().length).toBeGreaterThan(0)
      expect(metadata.description.length).toBeLessThanOrEqual(1024)
      expect(source.slice(match[0].length).split('\n').length).toBeLessThanOrEqual(500)
      return metadata.name
    })
    expect(new Set(names).size).toBe(names.length)
  })

  for (const filename of filesIn(skills).filter((file) => file.endsWith('.md'))) {
    it(`resolves local links in ${path.relative(skills, filename)}`, () => {
      const source = readFileSync(filename, 'utf8')
      for (const match of source.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
        const target = match[1].replace(/^<|>$/g, '').split('#')[0]
        if (!target || /^[a-z][a-z\d+.-]*:/i.test(target)) continue
        expect(
          existsSync(path.resolve(path.dirname(filename), decodeURIComponent(target))),
          `${path.relative(root, filename)} -> ${target}`,
        ).toBe(true)
      }
    })
  }

  it('keeps configured review-loop skills available', () => {
    const source = readFileSync(path.join(root, '.moatless/loops/code-review/LOOP.md'), 'utf8')
    const metadata = load(/^---\n([\s\S]*?)\n---/.exec(source)[1])
    for (const name of metadata.skills ?? []) {
      expect(existsSync(path.join(skills, name, 'SKILL.md')), name).toBe(true)
    }
  })

  it('parses skill YAML resources and keeps example image versions aligned', () => {
    for (const filename of filesIn(skills).filter((file) => /\.ya?ml$/.test(file))) {
      const values = load(readFileSync(filename, 'utf8'))
      if (!path.basename(filename).startsWith('values-')) continue
      expect(values.backend.image.tag).toBe(values.image.tag)
      expect(values.sandboxImage.tag).toBe(values.image.tag)
      expect(values.t3.image.tag).toBe(values.image.tag)
      expect(values.postgres.enabled).toBe(false)
      expect(values.database).toBeUndefined()
    }
  })
})

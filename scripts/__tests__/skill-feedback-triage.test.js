import { spawnSync } from 'node:child_process'
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'

const skill = path.resolve(import.meta.dirname, '../../.agents/skills/feedback-triage')
const stub = path.join(import.meta.dirname, 'fixtures/feedback-triage-moat.sh')
const stores = []
afterEach(() => {
  for (const store of stores.splice(0)) rmSync(store, { recursive: true, force: true })
})

function setup() {
  const store = mkdtempSync(path.join(tmpdir(), 'feedback-triage-'))
  stores.push(store)
  const bin = path.join(store, 'bin')
  spawnSync('mkdir', [bin])
  writeFileSync(path.join(bin, 'moat'), readFileSync(stub))
  chmodSync(path.join(bin, 'moat'), 0o755)
  const run = (command, args, { input, env } = {}) =>
    spawnSync(command, args, {
      input,
      encoding: 'utf8',
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, STORE: store, ...env },
    })
  const put = (key, data) =>
    run('moat', ['memory', 'put', key, '--scope', 'loop', '--summary', 'x', '--data', JSON.stringify(data)])
  const get = (key) => JSON.parse(run('moat', ['memory', 'get', key, '--json']).stdout).items[0]
  return { store, run, put, get }
}

const uuid = (n) => `${String(n).padStart(8, '0')}-1111-2222-3333-444444444444`
const held = { key: 'decisions/lost-state/1', updatedAt: 't1', decision: 'approve', held: true }
function heldCluster({ put, store }) {
  put('triage:cluster:lost-state', {
    slug: 'lost-state',
    title: 'State lost',
    owner: 'moatless',
    summary: 's',
    brief: 'b',
    members: null,
    allMembers: [uuid(1), uuid(2)],
    samples: [],
    status: 'proposed',
    decision: held,
  })
  const decision = path.join(store, 'decision.json')
  writeFileSync(decision, JSON.stringify({ ...held, slug: 'lost-state', login: 'a', note: '', pageSlug: 'p', taskId: 't' }))
  return decision
}
const hhmm = (offsetMinutes) => new Date(Date.now() + offsetMinutes * 60_000).toISOString().slice(11, 16)

describe('feedback-triage members.sh', () => {
  it('stores 400 ids in chunks under 16 KiB and reads back the full set with the legacy ones', () => {
    const { store, run, put } = setup()
    const ids = Array.from({ length: 400 }, (_, n) => uuid(n))
    put('triage:cluster:big', { slug: 'big', members: ids.slice(0, 5), allMembers: ids.slice(0, 100), allMembersKey: 'triage:extra:big' })
    put('triage:extra:big', { allMembers: ids.slice(100, 200) })

    const add = run(path.join(skill, 'scripts/members.sh'), ['add', 'big'], { input: JSON.stringify(ids.slice(150)) })
    expect(add.stderr).toBe('')
    expect(add.stdout.trim()).toBe('400')
    for (const n of [0, 1]) {
      const chunk = readFileSync(path.join(store, `triage%members%big%${n}.json`), 'utf8')
      expect(JSON.stringify(JSON.parse(chunk).data).length).toBeLessThanOrEqual(16 * 1024)
    }

    const read = run(path.join(skill, 'scripts/members.sh'), ['read', 'big'])
    expect(JSON.parse(read.stdout).sort()).toEqual(ids)
  })
})

describe('feedback-triage create-fix-task.sh', () => {
  it('builds the brief from allMembers when members is null, and retries a rate-limit 503', () => {
    const env = setup()
    const decision = heldCluster(env)
    writeFileSync(path.join(env.store, 'fails'), '1')

    const result = env.run(path.join(skill, 'scripts/create-fix-task.sh'), [decision], { env: { RETRY_AT: hhmm(-1) } })
    expect(result.status, result.stderr).toBe(0)
    expect(result.stdout.trim()).toBe('11111111-2222-3333-4444-555555555555')
    expect(result.stderr).toContain('rate-limited; retrying')
    expect(readFileSync(path.join(env.store, 'brief.md'), 'utf8')).toContain(`Member feedback ids: ${uuid(1)}, ${uuid(2)}`)
    expect(env.get('triage:cluster:lost-state').data).toMatchObject({ status: 'in-progress', childTaskId: result.stdout.trim() })
  }, 20_000)

  it('gives up without a task when the rate limit lasts past MAX_WAIT', () => {
    const env = setup()
    const decision = heldCluster(env)
    writeFileSync(path.join(env.store, 'fails'), '1')

    const result = env.run(path.join(skill, 'scripts/create-fix-task.sh'), [decision], {
      env: { RETRY_AT: hhmm(30), MAX_WAIT: '60' },
    })
    expect(result.status).toBe(1)
    expect(result.stderr).toContain(`rate-limited until ${hhmm(30)} UTC`)
    expect(env.get('triage:cluster:lost-state').data).toMatchObject({ status: 'proposed', decision: held })
  })
})

describe('feedback-triage step 3 PR match', () => {
  // The filter acting.md documents, so a change to it is tested as written.
  const acting = readFileSync(path.join(skill, 'references/acting.md'), 'utf8')
  const filter = /jq --arg l "Feedback-triage: \$slug" --arg since "\$acted_at" \\\n\s+'([\s\S]*?)' \\/.exec(acting)[1]
  const line = 'Feedback-triage: plugin-skills'
  const prs = [
    { number: 1, state: 'MERGED', createdAt: '2026-11-10T09:00:00Z', body: `Fix\n\n${line}\r\n` },
    { number: 2, state: 'OPEN', createdAt: '2026-11-25T09:00:00Z', body: `Again\n  ${line}` },
    { number: 3, state: 'MERGED', createdAt: '2026-11-25T10:00:00Z', body: `${line}-more` },
  ]

  it('ignores the merged PR of a reopened cluster from before its current task', () => {
    const { run } = setup()
    const result = run('jq', ['-c', '--arg', 'l', line, '--arg', 'since', '2026-11-24T12:00:00Z', `${filter} | map(.number)`], {
      input: JSON.stringify(prs),
    })
    expect(result.stdout.trim()).toBe('[2]')
  })
})

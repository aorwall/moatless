import type { UiMessage } from '@moatless/api-client'
import { describe, expect, it } from 'vitest'
import { groupSubagentMessages } from '../group-messages'

const message = (uuid: string, overrides: Partial<UiMessage> = {}): UiMessage => ({
  uuid,
  role: 'assistant',
  content: [],
  toolResults: [],
  messageType: 'assistant',
  timestamp: new Date().toISOString(),
  ...overrides,
})

const agentCall = (toolUseId: string) => ({
  type: 'tool_use' as const,
  toolUseId,
  toolName: 'Agent',
  toolInput: {},
})

const subagentMessage = (uuid: string, parentUuid: string, text: string): UiMessage =>
  message(uuid, { parentUuid, content: [{ type: 'text', text }] })

describe('groupSubagentMessages', () => {
  it('returns messages untouched when no Agent tool calls are present', () => {
    const messages = [message('a'), message('b')]
    expect(groupSubagentMessages(messages)).toEqual(messages)
  })

  it('attaches subagent messages to the Agent call that spawned them', () => {
    const parent = message('parent', { content: [agentCall('toolu_1')] })
    const child = subagentMessage('child', 'toolu_1', 'done')

    const grouped = groupSubagentMessages([parent, child])

    // The subagent message is lifted out of the main list and keyed by its call.
    expect(grouped).toHaveLength(1)
    expect(grouped[0]?.subagentsByToolUseId?.['toolu_1']).toHaveLength(1)
    expect(grouped[0]?.subagentsByToolUseId?.['toolu_1']?.[0]?.uuid).toBe('child')
  })

  it('keeps parallel Agent calls in one message separate', () => {
    // One assistant message launching two subagents at once — the shape the
    // backend now emits, since an API response is persisted as a single row.
    const parent = message('parent', {
      content: [agentCall('toolu_1'), agentCall('toolu_2')],
    })
    const first = subagentMessage('child-1', 'toolu_1', 'first result')
    const second = subagentMessage('child-2', 'toolu_2', 'second result')

    const grouped = groupSubagentMessages([parent, first, second])

    expect(grouped[0]?.subagentsByToolUseId?.['toolu_1']?.[0]?.uuid).toBe('child-1')
    expect(grouped[0]?.subagentsByToolUseId?.['toolu_2']?.[0]?.uuid).toBe('child-2')
  })

  it('groups every message belonging to one Agent call', () => {
    const parent = message('parent', { content: [agentCall('toolu_1')] })
    const first = subagentMessage('child-1', 'toolu_1', 'step one')
    const second = subagentMessage('child-2', 'toolu_1', 'step two')

    const grouped = groupSubagentMessages([parent, first, second])

    expect(grouped[0]?.subagentsByToolUseId?.['toolu_1']?.map((m) => m.uuid)).toEqual([
      'child-1',
      'child-2',
    ])
  })

  it('leaves messages whose parentUuid is not an Agent call in the main list', () => {
    const parent = message('parent', { content: [agentCall('toolu_1')] })
    // parentUuid pointing at an unrelated tool call must not be swallowed.
    const unrelated = subagentMessage('other', 'toolu_other', 'not a subagent')

    const grouped = groupSubagentMessages([parent, unrelated])

    expect(grouped.map((m) => m.uuid)).toEqual(['parent', 'other'])
  })
})

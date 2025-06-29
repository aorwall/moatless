import type { ToolUseContent } from '@shared/types/claude-content'
import type { Event, MessageEvent } from '@shared/types/event'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { EventList } from '../event-list'

// Mock the tool call components
const LSToolCallMock = ({ input }: { input: { path: string } }) => (
  <div data-testid="ls-tool-call">LS Tool Call: {input.path}</div>
)

vi.mock('../event-components/tool-calls/ls-tool-call', () => ({
  LSToolCall: LSToolCallMock,
}))

vi.mock('../event-components/tool-call-event', () => ({
  ToolCallEvent: ({ event }: { event: MessageEvent }) => {
    const payload = event.payload
    const content = payload?.message?.content || []
    const toolUse = content.find((item): item is ToolUseContent => item.type === 'tool_use')

    if (toolUse?.name === 'LS') {
      return <LSToolCallMock input={toolUse.input as { path: string }} />
    }

    return <div data-testid="tool-call-event">Generic Tool Call</div>
  },
}))

describe('EventList Tool Calls', () => {
  it('should render LS tool call with LSToolCall component', () => {
    const events: Event[] = [
      {
        eventId: 'c9703494-c72d-4e00-b564-9af669ca73e6',
        type: 'message',
        subType: 'tool_call',
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:32.511Z',
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01WJ4oGMtXCUrUKfRCcg2z4H',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'tool_use',
                id: 'toolu_01WyKnDTfzw1PfJaWyjQ8GUb',
                name: 'LS',
                input: {
                  path: '/Users/albert/repos/moatless/templates/bolt-workspace-7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
                },
              },
            ],
            stop_reason: null,
            stop_sequence: null,
            usage: {
              input_tokens: 4,
              cache_creation_input_tokens: 13502,
              cache_read_input_tokens: 0,
              output_tokens: 6,
              service_tier: 'standard',
            },
          },
          parent_tool_use_id: null,
          session_id: 'ed9da787-6508-413a-87bb-7c7ed8915897',
        },
      },
    ]

    render(<EventList events={events} />)

    // Should render the LS tool call
    expect(screen.getByTestId('ls-tool-call')).toBeInTheDocument()
    expect(
      screen.getByText(
        'LS Tool Call: /Users/albert/repos/moatless/templates/bolt-workspace-7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
      ),
    ).toBeInTheDocument()
  })

  it('should merge tool call with tool result', () => {
    const events: Event[] = [
      {
        eventId: 'c9703494-c72d-4e00-b564-9af669ca73e6',
        type: 'message',
        subType: 'tool_call',
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:32.511Z',
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01WJ4oGMtXCUrUKfRCcg2z4H',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'tool_use',
                id: 'toolu_01WyKnDTfzw1PfJaWyjQ8GUb',
                name: 'LS',
                input: {
                  path: '/Users/albert/repos/moatless/templates/bolt-workspace-7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
                },
              },
            ],
          },
        },
      },
      {
        eventId: '93d6016e-b264-4be3-b045-2fad6884abea',
        type: 'message',
        subType: 'tool_result',
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:33.137Z',
        isError: false,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                tool_use_id: 'toolu_01WyKnDTfzw1PfJaWyjQ8GUb',
                type: 'tool_result',
                content:
                  '- /Users/albert/repos/moatless/templates/bolt-workspace-7f5ddcd4-ca18-485a-9306-4b91ef6782d8/\n  - eslint.config.js\n  - index.html\n  - package-lock.json',
              },
            ],
          },
        },
      },
    ]

    render(<EventList events={events} />)

    // Should still render the LS tool call (merged events)
    expect(screen.getByTestId('ls-tool-call')).toBeInTheDocument()
  })

  it('should handle events without subType by checking content', () => {
    // This tests the current behavior where subType might not be set
    const events: Event[] = [
      {
        eventId: 'c9703494-c72d-4e00-b564-9af669ca73e6',
        type: 'message',
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:32.511Z',
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01WJ4oGMtXCUrUKfRCcg2z4H',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'text',
                text: 'Let me check the files in this directory.',
              },
            ],
          },
        },
      },
    ]

    render(<EventList events={events} />)

    // Should render as a regular message event
    expect(screen.queryByTestId('ls-tool-call')).not.toBeInTheDocument()
    expect(screen.queryByTestId('tool-call-event')).not.toBeInTheDocument()
  })
})

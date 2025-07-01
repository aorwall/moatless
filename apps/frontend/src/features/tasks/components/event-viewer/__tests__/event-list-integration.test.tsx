import type { Event } from '@moatless/shared/types/event'
import { EventType } from '@moatless/shared/types/event'
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { EventList } from '../event-list'

describe('EventList Integration - Tool Calls', () => {
  it('should render LS tool call with proper component', () => {
    const events: Event[] = [
      {
        eventId: 'c9703494-c72d-4e00-b564-9af669ca73e6',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: new Date('2025-06-25T12:38:32.511Z'),
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01WJ4oGMtXCUrUKfRCcg2z4H',
            type: EventType.MESSAGE,
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

    // Should render the LS tool name
    expect(screen.getByText('LS')).toBeInTheDocument()

    // Should show the ls command with path
    expect(
      screen.getByText(/ls \/Users\/albert\/repos\/moatless\/templates\/bolt-workspace/),
    ).toBeInTheDocument()

    // Should have the folder icon (part of LSToolCall component)
    const folderIcon = document.querySelector('svg.lucide-folder-open')
    expect(folderIcon).toBeInTheDocument()
  })

  it('should render tool call with result when merged', () => {
    const events: Event[] = [
      {
        eventId: 'c9703494-c72d-4e00-b564-9af669ca73e6',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: new Date('2025-06-25T12:38:32.511Z'),
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01WJ4oGMtXCUrUKfRCcg2z4H',
            type: EventType.MESSAGE,
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
          parent_tool_use_id: null,
          session_id: 'ed9da787-6508-413a-87bb-7c7ed8915897',
        },
      },
      {
        eventId: '93d6016e-b264-4be3-b045-2fad6884abea',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: new Date('2025-06-25T12:38:33.137Z'),
        isError: false,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'toolu_01WyKnDTfzw1PfJaWyjQ8GUb',
                content:
                  '- /Users/albert/repos/moatless/templates/bolt-workspace-7f5ddcd4-ca18-485a-9306-4b91ef6782d8/\n  - eslint.config.js\n  - index.html\n  - package-lock.json',
              },
            ],
          },
          parent_tool_use_id: null,
          session_id: 'ed9da787-6508-413a-87bb-7c7ed8915897',
        },
      },
    ]

    render(<EventList events={events} />)

    // Should render the LS tool
    expect(screen.getByText('LS')).toBeInTheDocument()

    // Should show the tool result
    expect(screen.getByText('Tool Result')).toBeInTheDocument()
    expect(screen.getByText(/eslint.config.js/)).toBeInTheDocument()
  })

  it('should render Read tool call', () => {
    const events: Event[] = [
      {
        eventId: 'b9e99242-2bef-4dc9-b47a-56028303a971',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: new Date('2025-06-25T12:38:37.552Z'),
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01CXrZbZ8ia8wwcoxJKnAQ64',
            type: EventType.MESSAGE,
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'tool_use',
                id: 'toolu_01FQfZbk5ZYnMxn1y3YGZffU',
                name: 'Read',
                input: {
                  filePath:
                    '/Users/albert/repos/moatless/templates/bolt-workspace-7f5ddcd4-ca18-485a-9306-4b91ef6782d8/src/App.tsx',
                },
              },
            ],
          },
          parent_tool_use_id: null,
          session_id: 'ed9da787-6508-413a-87bb-7c7ed8915897',
        },
      },
    ]

    render(<EventList events={events} />)

    // Should render the Read tool
    expect(screen.getByText('Read')).toBeInTheDocument()

    // Should show the file path
    expect(screen.getByText(/App\.tsx/)).toBeInTheDocument()
  })

  it('should render regular text messages without tool call styling', () => {
    const events: Event[] = [
      {
        eventId: 'e81f1d4c-d470-4bf0-bf53-8e20468fb922',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: new Date('2025-06-25T12:38:37.087Z'),
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01CXrZbZ8ia8wwcoxJKnAQ64',
            type: EventType.MESSAGE,
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'text',
                text: 'I see you have a React TypeScript project with Vite. Let me modify the App.tsx file to create a simple hello world app.',
              },
            ],
            stop_reason: null,
            stop_sequence: null,
            usage: {
              input_tokens: 6,
              cache_creation_input_tokens: 688,
              cache_read_input_tokens: 13502,
              output_tokens: 1,
              service_tier: 'standard',
            },
          },
          parent_tool_use_id: null,
          session_id: 'ed9da787-6508-413a-87bb-7c7ed8915897',
        },
      },
    ]

    render(<EventList events={events} />)

    // Should render the text content
    expect(
      screen.getByText(
        'I see you have a React TypeScript project with Vite. Let me modify the App.tsx file to create a simple hello world app.',
      ),
    ).toBeInTheDocument()

    // Should show Assistant label
    expect(screen.getByText('Assistant')).toBeInTheDocument()

    // Should not have tool-specific elements
    expect(screen.queryByText('Tool Result')).not.toBeInTheDocument()
  })

  it('should handle mixed content (text + tool use)', () => {
    const events: Event[] = [
      {
        eventId: 'mixed-content',
        agentType: 'claude-code',
        type: EventType.MESSAGE,
        taskId: 'test-task',
        iterationId: 1,
        timestamp: new Date('2025-06-25T12:38:37.087Z'),
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_mixed',
            type: EventType.MESSAGE,
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'text',
                text: 'Let me check the current directory structure:',
              },
              {
                type: 'tool_use',
                id: 'tool_123',
                name: 'LS',
                input: {
                  path: '/current/directory',
                },
              },
            ],
          },
          parent_tool_use_id: null,
          session_id: 'test-session',
        },
      },
    ]

    render(<EventList events={events} />)

    // Should render as a tool call (tool_use takes precedence)
    expect(screen.getByText('LS')).toBeInTheDocument()
    expect(screen.getByText(/ls \/current\/directory/)).toBeInTheDocument()
  })
})

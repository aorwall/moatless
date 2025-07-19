import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type { AgentMessage } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ClaudeService } from '../claude.service.js'

describe('ClaudeService AgentMessage Parsing', () => {
  let service: ClaudeService
  let mockSettingsService: any
  let tempDir: string
  let projectPath: string
  let sessionId: string

  beforeEach(async () => {
    // Create a temporary directory for testing
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'claude-test-'))
    projectPath = path.join(tempDir, 'projects', 'test-project')
    await fs.mkdir(projectPath, { recursive: true })

    sessionId = '0ef34def-db9a-4a85-9586-46df4167dce4'

    mockSettingsService = {
      getClaudeSettings: vi.fn().mockReturnValue({
        configDir: tempDir,
      }),
    }

    service = new ClaudeService(mockSettingsService)
  })

  afterEach(async () => {
    // Clean up temporary directory
    await fs.rm(tempDir, { recursive: true, force: true })
    vi.clearAllMocks()
  })

  it('should parse real-world JSON messages correctly', async () => {
    // The actual JSON lines from the user
    const jsonLines = [
      '{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/Users/albert/repos/moatless/moatless-swarm-ui","sessionId":"0ef34def-db9a-4a85-9586-46df4167dce4","version":"1.0.35","type":"user","message":{"role":"user","content":"commit changes"},"uuid":"b87215e7-8274-4a86-84c0-01998dd143cd","timestamp":"2025-06-27T09:05:45.614Z"}',
      '{"parentUuid":"b87215e7-8274-4a86-84c0-01998dd143cd","isSidechain":false,"userType":"external","cwd":"/Users/albert/repos/moatless/moatless-swarm-ui","sessionId":"0ef34def-db9a-4a85-9586-46df4167dce4","version":"1.0.35","message":{"id":"msg_01USKTt6JCUs8NSKExGdCHTv","type":"message","role":"assistant","model":"claude-opus-4-20250514","content":[{"type":"text","text":"I\'ll create a git commit with the current changes. Let me first check the status and see what changes need to be committed."}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":4,"cache_creation_input_tokens":8404,"cache_read_input_tokens":10164,"output_tokens":3,"service_tier":"standard"}},"requestId":"req_011CQYTbqesGwzdZ2KmpeKve","type":"assistant","uuid":"e127ad75-ca76-4278-9a44-9df0212bdc64","timestamp":"2025-06-27T09:05:50.606Z"}',
      '{"parentUuid":"e127ad75-ca76-4278-9a44-9df0212bdc64","isSidechain":false,"userType":"external","cwd":"/Users/albert/repos/moatless/moatless-swarm-ui","sessionId":"0ef34def-db9a-4a85-9586-46df4167dce4","version":"1.0.35","message":{"id":"msg_01USKTt6JCUs8NSKExGdCHTv","type":"message","role":"assistant","model":"claude-opus-4-20250514","content":[{"type":"tool_use","id":"toolu_011o12pQ7EEhYmKNCeawnPKN","name":"Bash","input":{"command":"git status","description":"Check current git status"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":4,"cache_creation_input_tokens":8404,"cache_read_input_tokens":10164,"output_tokens":3,"service_tier":"standard"}},"requestId":"req_011CQYTbqesGwzdZ2KmpeKve","type":"assistant","uuid":"d17a3e06-4bae-4105-b260-26e2594d6364","timestamp":"2025-06-27T09:05:52.318Z"}',
      '{"parentUuid":"d17a3e06-4bae-4105-b260-26e2594d6364","isSidechain":false,"userType":"external","cwd":"/Users/albert/repos/moatless/moatless-swarm-ui","sessionId":"0ef34def-db9a-4a85-9586-46df4167dce4","version":"1.0.35","message":{"id":"msg_01USKTt6JCUs8NSKExGdCHTv","type":"message","role":"assistant","model":"claude-opus-4-20250514","content":[{"type":"tool_use","id":"toolu_01Qugm9A5Pp9FhdUkmiF42do","name":"Bash","input":{"command":"git diff --cached","description":"View staged changes"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":4,"cache_creation_input_tokens":8404,"cache_read_input_tokens":10164,"output_tokens":3,"service_tier":"standard"}},"requestId":"req_011CQYTbqesGwzdZ2KmpeKve","type":"assistant","uuid":"65c16e85-cac1-4715-97f9-00aa38d19397","timestamp":"2025-06-27T09:05:53.382Z"}',
      '{"parentUuid":"5024ed1e-7c57-4c77-8313-c99461a389af","isSidechain":false,"userType":"external","cwd":"/Users/albert/repos/moatless/moatless-swarm-ui","sessionId":"0ef34def-db9a-4a85-9586-46df4167dce4","version":"1.0.35","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01Qugm9A5Pp9FhdUkmiF42do","type":"tool_result","content":"","is_error":false}]},"uuid":"00d372ae-09a6-441a-8fa3-42e01c952845","timestamp":"2025-06-27T09:05:54.383Z","toolUseResult":{"stdout":"","stderr":"","interrupted":false,"isImage":false}}',
    ]

    // Write the JSON lines to a file
    const messagesFile = path.join(projectPath, `${sessionId}.jsonl`)
    await fs.writeFile(messagesFile, jsonLines.join('\n'))

    // Call readClaudeMessages
    const messages = await service.readClaudeMessages(sessionId)

    // Verify we got all 5 messages
    expect(messages).toHaveLength(5)

    // Verify first message (user - commit changes)
    const firstMsg = messages[0]
    expect(firstMsg.type).toBe('user')
    expect(firstMsg.uuid).toBe('b87215e7-8274-4a86-84c0-01998dd143cd')
    expect(firstMsg.sessionId).toBe(sessionId)
    expect(firstMsg.parentUuid).toBeNull()
    expect(firstMsg.message.role).toBe('user')
    expect(firstMsg.message.content).toBe('commit changes')
    expect(firstMsg.timestamp).toBe('2025-06-27T09:05:45.614Z')

    // Verify second message (assistant - text response)
    const secondMsg = messages[1]
    expect(secondMsg.type).toBe('assistant')
    expect(secondMsg.uuid).toBe('e127ad75-ca76-4278-9a44-9df0212bdc64')
    expect(secondMsg.parentUuid).toBe('b87215e7-8274-4a86-84c0-01998dd143cd')
    expect(secondMsg.message.role).toBe('assistant')
    expect(secondMsg.message.model).toBe('claude-opus-4-20250514')
    expect(Array.isArray(secondMsg.message.content)).toBe(true)
    const textContent = secondMsg.message.content[0]
    expect(textContent.type).toBe('text')
    expect(textContent.text).toContain("I'll create a git commit")

    // Verify third message (assistant - tool use: git status)
    const thirdMsg = messages[2]
    expect(thirdMsg.type).toBe('assistant')
    expect(thirdMsg.uuid).toBe('d17a3e06-4bae-4105-b260-26e2594d6364')
    expect(thirdMsg.parentUuid).toBe('e127ad75-ca76-4278-9a44-9df0212bdc64')
    const toolUse1 = thirdMsg.message.content[0]
    expect(toolUse1.type).toBe('tool_use')
    expect(toolUse1.name).toBe('Bash')
    expect(toolUse1.input.command).toBe('git status')

    // Verify fourth message (assistant - tool use: git diff)
    const fourthMsg = messages[3]
    expect(fourthMsg.type).toBe('assistant')
    expect(fourthMsg.uuid).toBe('65c16e85-cac1-4715-97f9-00aa38d19397')
    const toolUse2 = fourthMsg.message.content[0]
    expect(toolUse2.type).toBe('tool_use')
    expect(toolUse2.name).toBe('Bash')
    expect(toolUse2.input.command).toBe('git diff --cached')

    // Verify fifth message (user - tool result)
    const fifthMsg = messages[4]
    expect(fifthMsg.type).toBe('user')
    expect(fifthMsg.uuid).toBe('00d372ae-09a6-441a-8fa3-42e01c952845')
    expect(fifthMsg.toolUseResult).toBeDefined()
    expect(fifthMsg.toolUseResult?.stdout).toBe('')
    expect(fifthMsg.toolUseResult?.interrupted).toBe(false)
    const toolResult = fifthMsg.message.content[0]
    expect(toolResult.type).toBe('tool_result')
    expect(toolResult.tool_use_id).toBe('toolu_01Qugm9A5Pp9FhdUkmiF42do')
    expect(toolResult.is_error).toBe(false)
  })

  it('should handle malformed JSON gracefully', async () => {
    const jsonLines = [
      // Valid JSON
      '{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/test","sessionId":"test-session","version":"1.0.35","type":"user","message":{"role":"user","content":"test"},"uuid":"test-123","timestamp":"2025-01-01T10:00:00.000Z"}',
      // Invalid JSON
      '{ broken json',
      // Another valid JSON
      '{"parentUuid":"test-123","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"test-session","version":"1.0.35","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"response"}]},"uuid":"test-456","timestamp":"2025-01-01T10:00:01.000Z"}',
    ]

    const messagesFile = path.join(projectPath, 'test-session.jsonl')
    await fs.writeFile(messagesFile, jsonLines.join('\n'))

    const messages = await service.readClaudeMessages('test-session')

    // Should only parse the valid messages
    expect(messages).toHaveLength(2)
    expect(messages[0].uuid).toBe('test-123')
    expect(messages[1].uuid).toBe('test-456')
  })

  it('should return empty array when file does not exist', async () => {
    const messages = await service.readClaudeMessages('non-existent-session')
    expect(messages).toEqual([])
  })

  it('should handle messages with all optional fields', async () => {
    const messageWithAllFields: AgentMessage = {
      parentUuid: 'parent-123',
      isSidechain: true,
      userType: 'internal',
      cwd: '/workspace',
      sessionId: 'full-session',
      version: '1.0.50',
      gitBranch: 'feature-branch',
      type: 'assistant',
      message: {
        id: 'msg-full',
        type: 'message',
        role: 'assistant',
        model: 'claude-3-opus',
        content: [{ type: 'text', text: 'Full message' }],
        stop_reason: 'stop_sequence',
        stop_sequence: '</thinking>',
        usage: {
          input_tokens: 100,
          output_tokens: 50,
          cache_creation_input_tokens: 20,
          cache_read_input_tokens: 30,
          server_tool_use: {
            web_search_requests: 2,
          },
          service_tier: 'pro',
        },
      },
      requestId: 'req-123',
      uuid: 'full-msg-uuid',
      timestamp: '2025-01-01T12:00:00.000Z',
    }

    const messagesFile = path.join(projectPath, 'full-session.jsonl')
    await fs.writeFile(messagesFile, JSON.stringify(messageWithAllFields))

    const messages = await service.readClaudeMessages('full-session')

    expect(messages).toHaveLength(1)
    const parsed = messages[0]
    expect(parsed.gitBranch).toBe('feature-branch')
    expect(parsed.requestId).toBe('req-123')
    expect(parsed.isSidechain).toBe(true)
    expect(parsed.message.usage?.server_tool_use?.web_search_requests).toBe(2)
  })
})

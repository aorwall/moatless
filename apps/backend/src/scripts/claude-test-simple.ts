import { query, type SDKMessage } from '@anthropic-ai/claude-agent-sdk'

console.log('Starting Claude Agent SDK test...')

try {
  const messages: SDKMessage[] = []

  for await (const message of query({
    prompt: 'hello',
    options: {
      maxTurns: 1,
      model: 'claude-sonnet-4',
      abortController: new AbortController(),
    },
  })) {
    messages.push(message)
  }

  console.log('Success! Messages received:', messages.length)
  console.log(JSON.stringify(messages, null, 2))
} catch (error) {
  console.error('Error occurred:', error)
}

import { query, type SDKMessage } from '@anthropic-ai/claude-code'

const messages: SDKMessage[] = []

for await (const message of query({
  prompt:
    "You're task is to determine a good branch name for the request 'login'. YOU MUST respond with ONLY the branch name.",
  abortController: new AbortController(),
  options: {
    maxTurns: 2,
    model: 'claude-sonnet-4-20250514',
  },
})) {
  messages.push(message)
}

console.log(JSON.stringify(messages, null, 2))

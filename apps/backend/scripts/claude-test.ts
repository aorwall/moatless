import { query, type SDKMessage } from '@anthropic-ai/claude-code'

const messages: SDKMessage[] = []

for await (const message of query({
  prompt:
    'Create a new TaskCard component, it will have the task name and repo and git status and if running show the minified events, otherwise the latest results. But think of a design where I can show these cards on a dashboard with many cards in a grid. Like it should look good in a pretty small view. Maybe you can make use of the @apps/frontend/src/features/tasks/contexts/task-providers.tsx that is not UI, like in @apps/frontend/src/features/tasks/components/task-page.tsx and then create a dashboard route where all not closed tasks will be shown. Like same as in @apps/frontend/src/features/navigation/components/sidebar-tasks.tsx .\n' +
    '\n' +
    '\n',
  abortController: new AbortController(),
  options: {
    customSystemPrompt:
      "You're task is to determine a good branch name for the request described. YOU MUST respond with ONLY the branch name. You can not use any tools, only respond directly.",
    maxTurns: 2,
    model: 'claude-sonnet-4-20250514',
  },
})) {
  messages.push(message)
}

console.log(JSON.stringify(messages, null, 2))

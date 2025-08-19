import { query, type SDKMessage } from '@anthropic-ai/claude-code'

const messages: SDKMessage[] = []

for await (const message of query({
  prompt:
    'You are tasked with creating a branch name and a summary for a given request. This information will be used in a version control system for tracking changes and organizing work. Your response must be in a strict JSON format.\n' +
    '\n' +
    'Guidelines for creating a good branch name:\n' +
    '- Use lowercase letters\n' +
    '- Replace spaces with hyphens\n' +
    '- Keep it concise but descriptive (ideally 2-4 words)\n' +
    '- Avoid special characters other than hyphens\n' +
    '- Start with a verb or category (e.g., "feature", "bugfix", "refactor")\n' +
    '\n' +
    'The request you need to analyze is provided below:\n' +
    '\n' +
    '<request>\n' +
    'Create a new TaskCard component, it will have the task name and repo and git status and if running show the minified events, otherwise the latest results. But think of a design where I can show these cards on a dashboard with many cards in a grid. Like it should look good in a pretty small view. Maybe you can make use of the @apps/frontend/src/features/tasks/contexts/task-providers.tsx that is not UI, like in @apps/frontend/src/features/tasks/components/task-page.tsx and then create a dashboard route where all not closed tasks will be shown. Like same as in @apps/frontend/src/features/navigation/components/sidebar-tasks.tsx .\n' +
    '</request>\n' +
    '\n' +
    'Analyze the request carefully. Based on the content, create a suitable branch name that follows the guidelines mentioned above. The branch name should reflect the main purpose or feature of the request.\n' +
    '\n' +
    'Next, create a brief summary of the task described in the request. This summary should be concise (1 sentence, max 10 words) but capture the essential elements of the task.\n' +
    '\n' +
    'Your final output must be in the following JSON format:\n' +
    '\n' +
    '<output>\n' +
    '{\n' +
    '  "branchName": "your-branch-name-here",\n' +
    '  "summary": "Your concise summary of the task here."\n' +
    '}\n' +
    '</output>\n' +
    '\n' +
    'Ensure that your response contains only the JSON object within the <output> tags. Do not include any additional text, explanations, or formatting outside of the JSON structure.',
  options: {
    appendSystemPrompt: '',
    maxTurns: 2,
    model: 'claude-sonnet-4-20250514',
    allowedTools: [],
    abortController: new AbortController(),
  },
})) {
  messages.push(message)
}

console.log(JSON.stringify(messages, null, 2))

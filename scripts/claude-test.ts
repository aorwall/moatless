import { query, type SDKMessage } from "@anthropic-ai/claude-code";

const messages: SDKMessage[] = [];

for await (const message of query({
    prompt: "I want to create a pet shop app",

    abortController: new AbortController(),
    options: {
        appendSystemPrompt: "You're task is to determine a good branch name for the users request. YOU MUST respond with ONLY a JSON object with the format  {branchName: []}",
        maxTurns: 1,
        allowedTools: [],
        model: "claude-sonnet-4-20250514",
    },
})) {
    messages.push(message);
}

console.log(JSON.stringify(messages, null, 2));

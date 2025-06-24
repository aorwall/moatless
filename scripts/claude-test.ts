import { query, type SDKMessage } from "@anthropic-ai/claude-code";

const messages: SDKMessage[] = [];

for await (const message of query({
    prompt: "Return a name for a  git branch about dogs, starting with 'feature/'",
    abortController: new AbortController(),
    options: {
        maxTurns: 3,
    },
})) {
    messages.push(message);
}

console.log(messages);

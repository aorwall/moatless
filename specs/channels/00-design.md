# Channel Integrations

Channels connect external messaging platforms (Slack, webhooks) to Moatless Vibe, allowing inbound messages to create tasks and outbound replies to flow back to the originating platform.

## What Users Can Do

- **Create a channel config** — link a provider (Slack or webhook) and a source identifier to a repository with a chosen routing mode.
- **View and list configs** — retrieve or browse channel configurations.
- **Update a config** — change the display name, prompt template, active status, or response routing.
- **Delete a config** — remove a channel configuration.
- **Route inbound messages** — messages arriving via webhook or Slack are automatically routed to create tasks or continue threads based on the routing mode.
- **Cross-channel routing** — link a trigger config to a separate response config so replies go to a different channel than the one that received the message.

## How It Works

1. A channel config binds a **provider + source** pair to a **repository**. The source identifier is provider-specific (e.g., a Slack channel ID or a webhook slug).
2. The **routing mode** controls how inbound messages map to tasks:
   - **per_message** — every message creates a new task.
   - **thread_based** — messages in the same thread continue the same task.
   - **ongoing** — all messages for a config go to a single long-lived task.
3. Outbound replies (agent responses, status updates) are sent back through the originating provider by default. If a **response config** is linked, replies go to that config's provider instead.
4. Configs can be activated or deactivated without deletion.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Channel config | A binding between a provider source and a repository |
| Provider | The external platform: Slack or webhook |
| Source | A provider-specific identifier (channel ID, webhook slug) |
| Routing mode | How inbound messages map to tasks: per_message, thread_based, or ongoing |
| Response config | An optional separate config that receives outbound replies |

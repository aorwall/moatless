# Figma Import

Figma import lets users bring design context into their tasks so the agent can reference visual designs when making code changes.

## What Users Can Do

- **Import from a Figma URL** — paste a Figma link and the system fetches the design context (screenshot, layout info, component mappings).
- **Import from clipboard** — paste Figma content directly; the system parses the clipboard payload.
- **Attach to a message** — imported design context is included as a content block in the conversation, giving the agent visual reference.

## How It Works

1. The frontend sends the Figma URL or clipboard data to the backend.
2. The backend calls the Figma API (via MCP or direct integration) to fetch screenshots, metadata, and code connect mappings.
3. The resulting design context is stored as a message attachment and displayed inline in the conversation.
4. The agent can reference the design when generating or modifying code — it sees the screenshot and any component hints.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Design context | A bundle of screenshot, layout metadata, and component mappings from Figma |
| Code Connect | Figma's mapping between design components and codebase components |
| Clipboard import | Parsing pasted Figma content without requiring a URL |

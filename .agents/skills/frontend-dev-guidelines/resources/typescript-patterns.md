# TypeScript conventions

Use generated API models for wire data and `import type` for type-only imports.
Prefer explicit props types and discriminated unions for states with different
required data. Narrow unknown values at boundaries; avoid `any` and assertions
that hide an unchecked response shape.

Use named exports for application modules. Framework entry points that require
a default export, including Storybook metadata and tool configuration, are exceptions.
Keep hooks unconditional, handle promises explicitly, and exhaustively handle
state variants. The root lint ratchet applies to touched source files.

Feature state and behavior belong under `features/<name>`, reusable primitives
under `lib`, and route files compose them. Use the root comment rules for TSDoc
contracts; do not narrate a function body in its doc comment.

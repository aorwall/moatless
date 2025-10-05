# Task Page Storybook Coverage

This note tracks the Storybook entries that exercise the task page layout and
workspace shell. Use these stories when iterating on styles or verifying
structure without wiring the full data layer.

## Layout Stories

- **`Features/Tasks/TaskPageLayout`** – renders the resizable two-column layout
  with mock header/chat/workspace content. The `ScrollableChat` story emits a
  tall message list to confirm scroll containment and footer positioning.

## Workspace Stories

- **`Features/Tasks/WorkspaceHeader`** – wraps the header in `TaskUIProvider` so
  the preview/files toggle can be exercised interactively.
- **`Features/Tasks/TaskWorkspacePanelView`** – showcases loading, error, files,
  and preview states of the right-hand workspace shell using slot props for the
  content panes.

## Usage

When building new widgets for either column, start from these stories to ensure
styling aligns with the shared layout primitives. They run without network
calls, so they are fast feedback loops for CSS and interaction tweaks.

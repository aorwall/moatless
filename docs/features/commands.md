# Commands Architecture

## Overview

The commands system provides a way to define and execute contextual actions that can be triggered by the user. These commands are surfaced in the UI based on the current state of a task and its workspace.

## Backend

### Command Service

The `CommandService` (`apps/backend/services/command.service.ts`) is responsible for managing commands. It loads a set of default commands and allows for dynamic creation, update, and deletion of commands.

### Command Conditions

Commands can have conditions that determine their availability. These conditions are evaluated by the `filterAvailableCommands` utility (`apps/backend/utils/command-conditions.ts`) based on the current task and workspace state.

Available conditions include:

- `requiresUncommittedChanges`: The workspace has uncommitted git changes.
- `requiresCommittedChanges`: The workspace has committed changes that have not been pushed.
- `requiresLastIterationError`: The last agent iteration resulted in an error.
- `allowedGitSyncStatuses`: The git sync status matches one of the allowed statuses.

## Frontend

### Command Buttons

The `CommandButtons` component (`apps/frontend/src/features/tasks/components/chat-view/command-buttons.tsx`) displays the available commands to the user. It fetches the available commands from the backend and renders them as buttons.

When a command button is clicked, its `prompt` is sent to the agent as a new message, triggering an agent iteration.

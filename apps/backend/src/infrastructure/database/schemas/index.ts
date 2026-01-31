/**
 * Schema registry for PostgreSQL database tables.
 *
 * Exports all table definitions for use in repositories and migrations.
 */

import { commands } from '../../../features/agents/db/command.schema'
import { apiKeys } from '../../../features/auth/db/api-key.schema'
import { oauthStates } from '../../../features/auth/db/oauth-state.schema'
import { providerTokens } from '../../../features/auth/db/provider-token.schema'
import { sessions } from '../../../features/auth/db/session.schema'
import { users } from '../../../features/auth/db/user.schema'
import { channelThreads } from '../../../features/channels/db/channel-thread.schema'
import { slackChannelMappings } from '../../../features/channels/db/slack-channel-mapping.schema'
import { taskEvents } from '../../../features/events/db/event.schema'
import { pluginMarketplaces } from '../../../features/plugins/db/plugin-marketplace.schema'
import { repositories } from '../../../features/repositories/db/repository.schema'
import { tags, taskTags } from '../../../features/tags/db/tag.schema'
import { taskIterations, tasks } from '../../../features/tasks/db/task.schema'
import { taskMessageContexts, taskMessages } from '../../../features/tasks/db/task-message.schema'
import { workspaceRuntimes } from '../../../features/workspace/db/workspace-runtime.schema'

/**
 * All database table schemas.
 */
export const schemas = {
  users,
  sessions,
  apiKeys,
  oauthStates,
  providerTokens,
  tasks,
  taskIterations,
  taskMessages,
  taskMessageContexts,
  taskEvents,
  repositories,
  pluginMarketplaces,
  workspaceRuntimes,
  commands,
  channelThreads,
  slackChannelMappings,
  tags,
  taskTags,
} as const

export type Schemas = typeof schemas

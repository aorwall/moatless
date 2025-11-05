PRAGMA foreign_keys=OFF;

DROP TABLE IF EXISTS `task_message_contexts`;

CREATE TABLE `task_message_contexts` (
  `id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
  `task_message_id` integer NOT NULL,
  `type` text NOT NULL,
  `reference_id` text NOT NULL,
  `data` text NOT NULL,
  `created_at` integer DEFAULT CURRENT_TIMESTAMP NOT NULL,
  FOREIGN KEY (`task_message_id`) REFERENCES `task_messages`(`id`) ON DELETE CASCADE
);

CREATE INDEX `task_message_contexts_message_idx` ON `task_message_contexts` (`task_message_id`);
CREATE INDEX `task_message_contexts_reference_idx` ON `task_message_contexts` (`reference_id`);

WITH context_values AS (
  SELECT
    tm.id AS task_message_id,
    json_extract(ctx.value, '$.type') AS context_type,
    json_extract(ctx.value, '$.referenceId') AS explicit_reference_id,
    json_extract(ctx.value, '$.data.componentId') AS element_reference_id,
    json_extract(ctx.value, '$.data.importId') AS figma_reference_id,
    json_extract(ctx.value, '$.data') AS data_json,
    tm.created_at AS created_at
  FROM `task_messages` tm
  JOIN json_each(tm.contexts) AS ctx
  WHERE tm.contexts IS NOT NULL
    AND json_valid(tm.contexts)
)
INSERT INTO `task_message_contexts` (
  `task_message_id`,
  `type`,
  `reference_id`,
  `data`,
  `created_at`
)
SELECT
  task_message_id,
  context_type,
  COALESCE(NULLIF(explicit_reference_id, ''), element_reference_id, figma_reference_id) AS reference_id,
  data_json,
  created_at
FROM context_values
WHERE context_type IN ('element', 'figma')
  AND COALESCE(NULLIF(explicit_reference_id, ''), element_reference_id, figma_reference_id) IS NOT NULL;

DROP TABLE IF EXISTS `task_messages_new`;

CREATE TABLE `task_messages_new` (
  `id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
  `task_id` text NOT NULL,
  `uuid` text NOT NULL,
  `message_type` text NOT NULL,
  `user_type` text NOT NULL,
  `parent_uuid` text,
  `is_sidechain` integer,
  `cwd` text,
  `session_id` text,
  `iteration_number` integer,
  `version` text,
  `git_branch` text,
  `request_id` text,
  `timestamp` text NOT NULL,
  `message_role` text NOT NULL,
  `message_content` text NOT NULL,
  `assistant_message_id` text,
  `assistant_message_type` text,
  `assistant_message_model` text,
  `assistant_stop_reason` text,
  `assistant_stop_sequence` text,
  `assistant_usage_input_tokens` integer,
  `assistant_usage_output_tokens` integer,
  `assistant_usage_cache_creation_input_tokens` integer,
  `assistant_usage_cache_read_input_tokens` integer,
  `tool_use_result` text,
  `attachments` text,
  `created_at` integer DEFAULT CURRENT_TIMESTAMP NOT NULL
);

INSERT INTO `task_messages_new` (
  `id`,
  `task_id`,
  `uuid`,
  `message_type`,
  `user_type`,
  `parent_uuid`,
  `is_sidechain`,
  `cwd`,
  `session_id`,
  `iteration_number`,
  `version`,
  `git_branch`,
  `request_id`,
  `timestamp`,
  `message_role`,
  `message_content`,
  `assistant_message_id`,
  `assistant_message_type`,
  `assistant_message_model`,
  `assistant_stop_reason`,
  `assistant_stop_sequence`,
  `assistant_usage_input_tokens`,
  `assistant_usage_output_tokens`,
  `assistant_usage_cache_creation_input_tokens`,
  `assistant_usage_cache_read_input_tokens`,
  `tool_use_result`,
  `attachments`,
  `created_at`
)
SELECT
  `id`,
  `task_id`,
  `uuid`,
  `message_type`,
  `user_type`,
  `parent_uuid`,
  `is_sidechain`,
  `cwd`,
  `session_id`,
  `iteration_number`,
  `version`,
  `git_branch`,
  `request_id`,
  `timestamp`,
  `message_role`,
  `message_content`,
  `assistant_message_id`,
  `assistant_message_type`,
  `assistant_message_model`,
  `assistant_stop_reason`,
  `assistant_stop_sequence`,
  `assistant_usage_input_tokens`,
  `assistant_usage_output_tokens`,
  `assistant_usage_cache_creation_input_tokens`,
  `assistant_usage_cache_read_input_tokens`,
  `tool_use_result`,
  `attachments`,
  `created_at`
FROM `task_messages`;

DROP TABLE `task_messages`;
ALTER TABLE `task_messages_new` RENAME TO `task_messages`;

CREATE INDEX `task_messages_task_idx` ON `task_messages` (`task_id`, `timestamp`, `id`);
CREATE UNIQUE INDEX `task_messages_uuid_idx` ON `task_messages` (`uuid`);

PRAGMA foreign_keys=ON;

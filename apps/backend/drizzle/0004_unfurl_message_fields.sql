PRAGMA foreign_keys=OFF;

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
  `contexts` text,
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
  `contexts`,
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
  CASE
    WHEN json_valid(`message`) THEN COALESCE(json_extract(`message`, '$.role'), `message_type`)
    ELSE `message_type`
  END,
  CASE
    WHEN json_valid(`message`) THEN COALESCE(
      json_extract(`message`, '$.content'),
      CASE WHEN `message_type` = 'assistant' THEN '[]' ELSE json_quote('') END
    )
    ELSE CASE WHEN `message_type` = 'assistant' THEN '[]' ELSE json_quote(COALESCE(`message`, '')) END
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.id')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN COALESCE(json_extract(`message`, '$.type'), 'message')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.model')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.stop_reason')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.stop_sequence')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.usage.input_tokens')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.usage.output_tokens')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.usage.cache_creation_input_tokens')
    ELSE NULL
  END,
  CASE
    WHEN `message_type` = 'assistant' AND json_valid(`message`) THEN json_extract(`message`, '$.usage.cache_read_input_tokens')
    ELSE NULL
  END,
  `tool_use_result`,
  `attachments`,
  `contexts`,
  `created_at`
FROM `task_messages`;

DROP TABLE `task_messages`;
ALTER TABLE `task_messages_new` RENAME TO `task_messages`;

CREATE INDEX `task_messages_task_idx` ON `task_messages` (`task_id`, `timestamp`, `id`);
CREATE UNIQUE INDEX `task_messages_uuid_idx` ON `task_messages` (`uuid`);

PRAGMA foreign_keys=ON;

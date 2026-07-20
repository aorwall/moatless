-- Drop the Figma import feature.
DROP TABLE IF EXISTS figma_imports;
DELETE FROM task_message_contexts WHERE context_type = 'figma';

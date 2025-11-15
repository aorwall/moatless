-- Add Moatable template repository
-- Inserts a pre-configured template repository for the Moatable project
-- with preview configuration including auto-reload, inspector, and screenshots

INSERT INTO repositories (
  id,
  kind,
  name,
  description,
  remote_url,
  provider,
  setup_commands,
  server_start_commands,
  auto_reload,
  enable_inspector,
  take_screenshots,
  build_output_directory,
  startup_timeout_seconds,
  created_at,
  updated_at
) VALUES (
  '7b8cf8f2-f3f5-4396-ab47-9e4d4e72ff51',
  'template',
  'Moatable',
  'Moatable template repository',
  'https://github.com/aorwall/moatable',
  'github',
  '["bun install"]',
  '["bun run dev"]',
  1,
  1,
  1,
  'dist',
  30,
  strftime('%s', 'now') * 1000,
  strftime('%s', 'now') * 1000
);

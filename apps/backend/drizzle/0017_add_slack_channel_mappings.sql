-- Add slack_channel_mappings table for mapping Slack channels to default repositories
CREATE TABLE slack_channel_mappings (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  channel_id TEXT NOT NULL,
  team_id TEXT NOT NULL,
  repository_id TEXT NOT NULL,
  created_by TEXT NOT NULL,
  created_at INTEGER DEFAULT (unixepoch()) NOT NULL,
  updated_at INTEGER DEFAULT (unixepoch()) NOT NULL
);--> statement-breakpoint

CREATE UNIQUE INDEX slack_channel_mappings_channel_team_idx ON slack_channel_mappings (channel_id, team_id);

-- Add slack_channel_mappings table for mapping Slack channels to default repositories
CREATE TABLE "slack_channel_mappings" (
	"id" serial PRIMARY KEY NOT NULL,
	"channel_id" text NOT NULL,
	"team_id" text NOT NULL,
	"repository_id" text NOT NULL,
	"created_by" text NOT NULL,
	"created_at" timestamp DEFAULT now() NOT NULL,
	"updated_at" timestamp DEFAULT now() NOT NULL
);

CREATE UNIQUE INDEX "slack_channel_mappings_channel_team_idx" ON "slack_channel_mappings" USING btree ("channel_id", "team_id");

-- Migration: Add plugin marketplaces table and plugins column to repositories
-- This migration adds support for plugin marketplace integration

-- Create plugin_marketplaces table
CREATE TABLE IF NOT EXISTS "plugin_marketplaces" (
  "id" text PRIMARY KEY NOT NULL,
  "name" text NOT NULL UNIQUE,
  "repo_url" text NOT NULL,
  "owner_name" text,
  "description" text,
  "last_synced_at" timestamp,
  "created_at" timestamp NOT NULL,
  "updated_at" timestamp NOT NULL
);

-- Create index on marketplace name for lookups
CREATE INDEX IF NOT EXISTS "plugin_marketplaces_name_idx" ON "plugin_marketplaces" ("name");

-- Add plugins column to repositories table
ALTER TABLE "repositories" ADD COLUMN IF NOT EXISTS "plugins" jsonb;

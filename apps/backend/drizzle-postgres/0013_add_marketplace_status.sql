-- Migration: Add status and last_error columns to plugin_marketplaces
-- This enables tracking marketplace sync state and displaying error messages

ALTER TABLE "plugin_marketplaces" ADD COLUMN IF NOT EXISTS "status" text NOT NULL DEFAULT 'unknown';
ALTER TABLE "plugin_marketplaces" ADD COLUMN IF NOT EXISTS "last_error" text;

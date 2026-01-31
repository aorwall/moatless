-- Add servers column to repositories table for multi-server configuration
ALTER TABLE "repositories" ADD COLUMN "servers" jsonb DEFAULT '[]'::jsonb;

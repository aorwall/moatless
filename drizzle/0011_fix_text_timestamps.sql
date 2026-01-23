-- Fix text timestamps to integer timestamps for template repositories
-- Converts ISO 8601 date strings to Unix timestamps in milliseconds

-- Update hello-world-ui template
UPDATE repositories
SET
  created_at = CAST(strftime('%s', created_at) AS INTEGER) * 1000,
  updated_at = CAST(strftime('%s', updated_at) AS INTEGER) * 1000
WHERE id = 'hello-world-ui' AND typeof(created_at) = 'text';--> statement-breakpoint

-- Update figma-prototype template
UPDATE repositories
SET
  created_at = CAST(strftime('%s', created_at) AS INTEGER) * 1000,
  updated_at = CAST(strftime('%s', updated_at) AS INTEGER) * 1000
WHERE id = 'figma-prototype' AND typeof(created_at) = 'text';--> statement-breakpoint

-- Update revealjs-presentation template
UPDATE repositories
SET
  created_at = CAST(strftime('%s', created_at) AS INTEGER) * 1000,
  updated_at = CAST(strftime('%s', updated_at) AS INTEGER) * 1000
WHERE id = 'revealjs-presentation' AND typeof(created_at) = 'text';--> statement-breakpoint

-- Fix any other repositories that might have text timestamps
UPDATE repositories
SET
  created_at = CAST(strftime('%s', created_at) AS INTEGER) * 1000,
  updated_at = CAST(strftime('%s', updated_at) AS INTEGER) * 1000
WHERE typeof(created_at) = 'text';

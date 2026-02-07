-- Fix files server port to 4000 (matches sandbox container's file-explorer)
-- Previous migrations created files server without explicit port, so assignMissingPorts()
-- gave it port 3000. The sandbox container runs file-explorer on port 4000.

UPDATE repositories
SET servers = (
  SELECT jsonb_agg(
    CASE
      WHEN elem->>'name' = 'files'
      THEN jsonb_set(elem, '{port}', '4000')
      ELSE elem
    END
  )
  FROM jsonb_array_elements(servers) AS elem
),
updated_at = NOW()
WHERE EXISTS (
  SELECT 1 FROM jsonb_array_elements(servers) AS elem
  WHERE elem->>'name' = 'files' AND (elem->'port' IS NULL OR (elem->>'port')::int != 4000)
);

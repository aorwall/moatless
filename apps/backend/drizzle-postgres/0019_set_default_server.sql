-- Set "preview" server as default for repos that have one
UPDATE repositories
SET servers = (
  SELECT jsonb_agg(
    CASE
      WHEN elem->>'name' = 'preview'
      THEN elem || '{"default": true}'::jsonb
      ELSE elem - 'default'
    END ORDER BY idx
  )
  FROM jsonb_array_elements(servers) WITH ORDINALITY AS t(elem, idx)
),
updated_at = NOW()
WHERE EXISTS (
  SELECT 1 FROM jsonb_array_elements(servers) AS elem
  WHERE elem->>'name' = 'preview'
);

-- Set "files" server as default for repos that have NO preview server
UPDATE repositories
SET servers = (
  SELECT jsonb_agg(
    CASE
      WHEN elem->>'name' = 'files'
      THEN elem || '{"default": true}'::jsonb
      ELSE elem - 'default'
    END ORDER BY idx
  )
  FROM jsonb_array_elements(servers) WITH ORDINALITY AS t(elem, idx)
),
updated_at = NOW()
WHERE NOT EXISTS (
  SELECT 1 FROM jsonb_array_elements(servers) AS elem
  WHERE elem->>'name' = 'preview'
)
AND jsonb_array_length(COALESCE(servers, '[]'::jsonb)) > 0;

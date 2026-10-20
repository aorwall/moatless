-- Every Artifact record is a value under a key: a save under an existing key
-- replaces it. A record saved without one keeps its id as its key.

UPDATE artifact_records SET key = record_id::text WHERE key IS NULL;
ALTER TABLE artifact_records ALTER COLUMN key SET NOT NULL;
ALTER TABLE artifact_records DROP CONSTRAINT IF EXISTS artifact_records_pkey;
ALTER TABLE artifact_records DROP COLUMN IF EXISTS record_id;
DROP INDEX IF EXISTS artifact_records_key_idx;
ALTER TABLE artifact_records ADD PRIMARY KEY (artifact_id, key);

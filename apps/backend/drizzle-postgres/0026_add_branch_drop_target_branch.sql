ALTER TABLE tasks ADD COLUMN branch text;
ALTER TABLE tasks DROP COLUMN target_branch;

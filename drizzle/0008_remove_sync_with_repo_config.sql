-- Drop sync_with_repo_config column from repositories table
-- Config files are now always read automatically during repository creation
ALTER TABLE repositories DROP COLUMN sync_with_repo_config;

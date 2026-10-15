-- Every statement on `task_forks` filters by `task_id`, which the primary key
-- serves; "what was forked off this Task" reads the `forked_from` edge in
-- `task_relations`. The index only cost a write per fork or rewind.
DROP INDEX IF EXISTS task_forks_source_idx;

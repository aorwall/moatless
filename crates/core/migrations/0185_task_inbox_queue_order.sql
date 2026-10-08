-- Where a person moved a pending `queue` item among its Task's others. `NULL`
-- keeps arrival order; a reorder renumbers every pending queue item of the
-- Task, so a later arrival sorts after them (`inbox/dao.rs::pending`).
ALTER TABLE task_inbox ADD COLUMN IF NOT EXISTS queue_order INTEGER;

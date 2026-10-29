-- The Task Room a Loop's spawned Tasks get (`moatless_types::loops::RoomSpec`,
-- `LOOP.md`'s `room` key). NULL opens none.
ALTER TABLE loop_configs ADD COLUMN IF NOT EXISTS room jsonb;

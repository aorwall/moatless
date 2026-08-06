-- One word for the record: `adapter_work` becomes `adapter_delivery`.
--
-- "Work" was the residual name — correct at the element level but the least
-- informative word available, kept only because everything more specific was
-- blocked. The record exists to say what the backend owes a sender for one
-- inbound delivery, so it is named for that. The `adapter_` prefix keeps our
-- record distinct from the sender's own delivery, the same way it already does
-- for an Adapter connection.
--
-- Cardinality, stated here because it is the cost of the name: one sender
-- delivery becomes *one or more* rows — one per (adapter, kind, event_id) — so
-- a GitHub delivery that fans into a message and a merge-conflict check is two
-- rows that retry and fail independently.
--
-- The `provider` column goes with it. The glossary forbids "provider" as a
-- standalone word (it is only ever a qualifier, as in "provider token"); the
-- value in this column has always been an Adapter name, so the column is
-- `adapter`.
--
-- A compatibility view over the old name was considered and rejected: admission
-- inserts with an explicit `ON CONFLICT (adapter, kind, event_id)` target, which
-- resolves against a real unique constraint that a view does not have, so every
-- admission through such a view would fail outright. Nothing needs one here in
-- any case — the system is pre-production, so there is no rolling restart to
-- survive, no live sender to fail, and no accumulated records to protect.

ALTER TABLE "adapter_work" RENAME TO "adapter_delivery";
ALTER TABLE "adapter_delivery" RENAME COLUMN "provider" TO "adapter";

ALTER TABLE "adapter_delivery" RENAME CONSTRAINT "adapter_work_status_check"
    TO "adapter_delivery_status_check";

ALTER INDEX IF EXISTS "adapter_work_pkey" RENAME TO "adapter_delivery_pkey";
-- Idempotent admission: a duplicate delivery of the same Adapter event for the
-- same kind conflicts here and is acknowledged without creating a second row.
ALTER INDEX IF EXISTS "adapter_work_event_idx" RENAME TO "adapter_delivery_event_idx";
-- The capacity predicate's partial index, matched exactly to admission's count.
ALTER INDEX IF EXISTS "adapter_work_unfinished_idx" RENAME TO "adapter_delivery_unfinished_idx";
-- The worker's claim scan: due pending records, oldest first.
ALTER INDEX IF EXISTS "adapter_work_due_idx" RENAME TO "adapter_delivery_due_idx";
-- Stale-claim recovery scan.
ALTER INDEX IF EXISTS "adapter_work_claimed_idx" RENAME TO "adapter_delivery_claimed_idx";
-- Retention pruning of finished records.
ALTER INDEX IF EXISTS "adapter_work_finished_idx" RENAME TO "adapter_delivery_finished_idx";

COMMENT ON COLUMN "adapter_delivery"."adapter" IS
    'The Adapter this delivery arrived on ("slack", "telegram", "github", "linear", "webhook").';
COMMENT ON COLUMN "adapter_delivery"."event_id" IS
    'The sender''s own delivery identifier, never one the backend generates — except for the generic Webhook Adapter, whose caller may supply one and gets a freshly minted one when it does not.';

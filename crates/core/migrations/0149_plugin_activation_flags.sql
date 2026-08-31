-- Two flags an activation record carries beside its on/off decision.
--
-- `always` lifts a record above every record that is not `always`, whatever
-- reach either of them carries, so a skill turned on for everyone or for one
-- person keeps being delivered into a Workspace that never named it — and into
-- one whose own record would otherwise have turned it off.
--
-- `frontload` asks for the skill's body to lead every fresh agent session on
-- the tasks it reaches, rather than waiting to be picked for a turn.
--
-- Both default false, which is what every row written before them meant.

ALTER TABLE "plugin_activations"
    ADD COLUMN IF NOT EXISTS "always" boolean NOT NULL DEFAULT false;

ALTER TABLE "plugin_activations"
    ADD COLUMN IF NOT EXISTS "frontload" boolean NOT NULL DEFAULT false;

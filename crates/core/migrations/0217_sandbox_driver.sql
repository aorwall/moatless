-- The person whose message started the Sandbox's current turn: its credentials
-- resolve for them, and its task tokens are minted for them. NULL until a
-- person other than the owner drives it, and read as the owner while NULL.
-- Every change bumps `token_generation` in the same statement.
ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS driver_user_id text;

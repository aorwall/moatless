-- The generation of the Sandbox's task tokens: each token carries the value it
-- was minted at as its `gen` claim, and a token whose claim differs is refused.
-- Bumping it revokes every token minted before. A token minted before the claim
-- existed reads as 0, this default.
ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS token_generation bigint NOT NULL DEFAULT 0;

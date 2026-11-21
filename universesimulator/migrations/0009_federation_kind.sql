-- What the entity that governs supply is, as players read it: a corporation, a federation, an
-- organisation... A slug the game names in the player's language (ADR 0016). ARES is a corporation.
ALTER TABLE federations ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'federation'
    CHECK (kind ~ '^[a-z][a-z0-9_]{0,31}$');

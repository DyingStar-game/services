-- A village whose depots left the world (a purge of the persisted world: a depot is back only once its
-- village is loaded again) is no longer simulated: `active` goes off and `gone_at` says the import did
-- it, so that the import, and only it, turns it on again when the depots are back. A village an admin
-- turned off has no `gone_at` and stays off.
ALTER TABLE villages ADD COLUMN IF NOT EXISTS gone_at TIMESTAMPTZ;

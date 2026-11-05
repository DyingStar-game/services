-- 0004_mission_giver
--
-- In-fiction giver of a mission (faction + optional division, free slugs owned by the
-- game's catalogue). Null on every existing row: the game falls back on `issuer_type`.
ALTER TABLE "missions" ADD COLUMN "giver_faction" text;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "giver_division" text;

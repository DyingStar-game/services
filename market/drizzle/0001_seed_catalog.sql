-- Bootstrap of the tradable goods list, so a fresh database never serves an empty
-- `GET /api/market/catalog` (the game UI reads it to build its goodType selector).
--
-- `good_type` and `kind` mirror `inventory.inventory_good_types`, which is already
-- authoritative for those two columns; `unit` and `display_name` are the market's own.
-- This is a floor, not an authority: the game server's `PUT /api/internal/catalog/:goodType`
-- is an upsert and overwrites these rows. `ON CONFLICT DO NOTHING` keeps the seed from
-- clobbering an entry the game already pushed. `enabled` and the timestamps take their
-- defaults (true / now()).
INSERT INTO "market_catalog" ("good_type", "kind", "unit", "display_name")
VALUES
  ('ore.gold', 'stack', 'litre', 'Gold'),
  ('ore.iron', 'stack', 'litre', 'Iron'),
  ('ore.cryptonite', 'stack', 'litre', 'Cryptonite')
ON CONFLICT ("good_type") DO NOTHING;

-- The ore chain of the game (2026-11-06): a factory city needs INGOTS, refined from the crushed ore
-- a mining village ships. A refined resource names its input; a shortage of it is met by shipping the
-- input (refined at the destination), or by a single mission that mines it and delivers the result.
ALTER TABLE resources ADD COLUMN refined_from TEXT REFERENCES resources(code) ON UPDATE CASCADE;

ALTER TABLE supply_orders DROP CONSTRAINT supply_orders_kind_valid;
ALTER TABLE supply_orders ADD CONSTRAINT supply_orders_kind_valid
    CHECK (kind IN ('transport', 'mining', 'mining_transport'));

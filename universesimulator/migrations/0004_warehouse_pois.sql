-- The POI `inventory` holds for a warehouse, created the first time a mission points at it:
-- `mission` resolves its objectives' places in `inventory`, which makes up its own POI ids.
ALTER TABLE warehouses ADD COLUMN inventory_poi_id UUID;

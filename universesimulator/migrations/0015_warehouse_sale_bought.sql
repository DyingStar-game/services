-- Off-mission sales stop at the warehouse's capacity (ADR 0017, amended 2026-11-19): what of a sale's
-- quantity was bought. Every sale made before bought all of it.
ALTER TABLE warehouse_sales ADD COLUMN IF NOT EXISTS bought BIGINT;
UPDATE warehouse_sales SET bought = quantity WHERE bought IS NULL;
ALTER TABLE warehouse_sales ALTER COLUMN bought SET NOT NULL;
ALTER TABLE warehouse_sales DROP CONSTRAINT IF EXISTS warehouse_sales_bought_check;
ALTER TABLE warehouse_sales ADD CONSTRAINT warehouse_sales_bought_check CHECK (bought >= 0 AND bought <= quantity);

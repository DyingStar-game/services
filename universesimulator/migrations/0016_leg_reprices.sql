-- Offered missions follow the current prices (ADR 0021): a leg whose untaken mission is cancelled to be
-- offered again at the current reward is a new attempt (a new scriptId), but not a failure. `reprices`
-- counts those attempts, which SIM_LEG_MAX_ATTEMPTS does not count.
ALTER TABLE supply_order_legs ADD COLUMN IF NOT EXISTS reprices INT NOT NULL DEFAULT 0;

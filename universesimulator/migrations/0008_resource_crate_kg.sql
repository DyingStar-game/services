-- What a crate of the resource typically weighs in the game (kg): a truck carries a fixed number of
-- crates (simulation::tiers::TRUCK_CRATES), so a truckload, the unit of the mission tiers, depends on it.
ALTER TABLE resources ADD COLUMN IF NOT EXISTS crate_kg DOUBLE PRECISION NOT NULL DEFAULT 25 CHECK (crate_kg > 0);

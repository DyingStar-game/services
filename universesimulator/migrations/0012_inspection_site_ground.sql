-- An inspection site is made at its village's altitude, the simulator knowing no relief, and gets no
-- mission before the game server, which has the terrain, has laid it on the ground (ADR 0019).
ALTER TABLE inspection_sites ADD COLUMN IF NOT EXISTS grounded_at TIMESTAMPTZ;

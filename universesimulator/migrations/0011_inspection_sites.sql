-- Inspections (ADR 0019): the places around a village its security has inspected, and the mission open
-- on one of them. A site is made once (a point 5 to 40 km from its village, its POI in `inventory`) and
-- inspected again and again: no place is created per mission.
CREATE TABLE IF NOT EXISTS inspection_sites (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    federation_id UUID NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    village_id UUID NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    seq INT NOT NULL,
    name TEXT NOT NULL,
    x DOUBLE PRECISION NOT NULL,
    y DOUBLE PRECISION NOT NULL,
    z DOUBLE PRECISION NOT NULL,
    distance_m DOUBLE PRECISION NOT NULL,
    inventory_poi_id UUID,
    -- The mission open on the site, NULL when none; `attempts` counts those it had (its missions' keys).
    mission_id UUID,
    published_at TIMESTAMPTZ,
    attempts INT NOT NULL DEFAULT 0,
    last_inspected_at TIMESTAMPTZ,
    UNIQUE (village_id, seq)
);

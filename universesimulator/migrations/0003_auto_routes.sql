-- Routes created by the import (nearest neighbours, world-file extras) are recomputed at each
-- import or tick replay; routes created by an admin are never removed by it (ADR 0013).
ALTER TABLE travel_routes ADD COLUMN auto BOOLEAN NOT NULL DEFAULT FALSE;

-- Every route created before this column existed came from the import.
UPDATE travel_routes SET auto = TRUE;

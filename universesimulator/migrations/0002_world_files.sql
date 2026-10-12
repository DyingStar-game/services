-- Last imported world file per federation: the tick replays it periodically so that villages
-- newly linked to a cargo_depot are simulated without a manual import (ADR 0013).
CREATE TABLE world_files (
    federation_id  UUID        PRIMARY KEY REFERENCES federations(id) ON DELETE CASCADE,
    content        JSONB       NOT NULL,
    imported_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

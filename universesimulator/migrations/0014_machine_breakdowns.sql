-- Breakdowns (ADR 0020). The game server says which machines stand in the villages (`machines`, its
-- objects' ids); the simulator breaks one now and then and offers its repair as a mission
-- (`machine_breakdowns`). A machine stays broken until the game server says a player repaired it.
CREATE TABLE IF NOT EXISTS machines (
    id UUID PRIMARY KEY,
    federation_id UUID NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    village_id UUID NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    -- A slug the game gives: crusher, refinery, dispenser, lift, teleporter...
    kind TEXT NOT NULL,
    -- Last time the game server listed it: a machine not seen for long is not broken.
    seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_repaired_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS machine_breakdowns (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    machine_id UUID NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
    broken_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    repaired_at TIMESTAMPTZ,
    repaired_by UUID,
    -- The mission open on the repair, NULL when none; `attempts` counts those it had (its missions' keys).
    mission_id UUID,
    published_at TIMESTAMPTZ,
    attempts INT NOT NULL DEFAULT 0
);

-- A machine is broken once at a time.
CREATE UNIQUE INDEX IF NOT EXISTS machine_breakdowns_open ON machine_breakdowns (machine_id) WHERE repaired_at IS NULL;

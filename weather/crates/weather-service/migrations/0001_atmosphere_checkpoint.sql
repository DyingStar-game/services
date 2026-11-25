-- Checkpoints of the atmosphere model (crate weather-atmosphere): two rolling slots per body,
-- written in turn. Losing them is harmless: the model makes a cold start (spin-up) again.
CREATE TABLE atmosphere_checkpoint (
    -- Body name in lower case ("sandbox").
    body        TEXT             NOT NULL,
    -- Rolling slot, 0 or 1.
    slot        SMALLINT         NOT NULL CHECK (slot IN (0, 1)),
    -- Model time of the saved state (s since the model's start).
    model_time  DOUBLE PRECISION NOT NULL,
    -- True when the guard accepted the state: only healthy states are loaded back.
    healthy     BOOLEAN          NOT NULL,
    saved_at    TIMESTAMPTZ      NOT NULL DEFAULT now(),
    -- The rest of the checkpoint's metadata (chunk, spin-up done, pacing anchor), as JSON.
    meta        JSONB            NOT NULL,
    -- The full model state, bit-exact (format DSPE0003, 1 to 10 MB).
    state       BYTEA            NOT NULL,
    PRIMARY KEY (body, slot)
);

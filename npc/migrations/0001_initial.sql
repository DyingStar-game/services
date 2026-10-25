-- NPC service: the roster (one row per NPC, each a Keycloak user), the shift
-- schedules, the manual tasks and the key events shown in the admin panel.

-- Service-wide switches. A single row (id is always TRUE).
CREATE TABLE settings (
    id            BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (id),
    -- Global switch: when FALSE no NPC is connected to Horizon, whatever its own `enabled`.
    npcs_enabled  BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by    TEXT
);
INSERT INTO settings DEFAULT VALUES;

-- Shift patterns: one 25-character line of planetary hours per rota-day
-- (T work, t transmission, r meal, a mandatory, . off, l leisure).
-- Imported from config/schedules.yaml, then edited from the admin panel.
CREATE TABLE schedule_patterns (
    name         TEXT        PRIMARY KEY CHECK (name ~ '^[a-z0-9_]{1,64}$'),
    description  TEXT,
    days         TEXT[]      NOT NULL CHECK (cardinality(days) BETWEEN 1 AND 14),
    source       TEXT        NOT NULL DEFAULT 'panel' CHECK (source IN ('import', 'panel')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by   TEXT
);

-- Activity -> actions the schedule allows during it (`activity_actions` of the YAML).
-- `always` holds the ambient actions allowed in every activity.
CREATE TABLE activity_actions (
    activity    TEXT        PRIMARY KEY CHECK (activity IN ('work', 'transmission', 'meal', 'mandatory', 'off', 'always')),
    actions     TEXT[]      NOT NULL DEFAULT '{}',
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by  TEXT
);

CREATE TABLE npcs (
    -- Keycloak user id = JWT `sub` = player id on Horizon.
    id                UUID        PRIMARY KEY,
    login             TEXT        NOT NULL UNIQUE,
    -- nonce (12 bytes) || AES-256-GCM ciphertext, AAD = id. Needed in clear for the password grant.
    password_enc      BYTEA       NOT NULL,
    password_key_id   SMALLINT    NOT NULL DEFAULT 1,
    first_name        TEXT        NOT NULL,
    last_name         TEXT        NOT NULL,
    display_name      TEXT        NOT NULL,
    -- Desired state: should this NPC be connected (when the global switch is on)?
    enabled           BOOLEAN     NOT NULL DEFAULT FALSE,
    role              TEXT        NOT NULL DEFAULT 'unassigned'
                                  CHECK (role IN ('unassigned', 'storekeeper', 'villager', 'custom')),
    -- Set by an admin: the automatic assignment at first spawn leaves this NPC alone.
    role_locked       BOOLEAN     NOT NULL DEFAULT FALSE,
    village_code      TEXT,
    storekeeper_slot  SMALLINT    CHECK (storekeeper_slot BETWEEN 1 AND 5),
    schedule_pattern  TEXT        REFERENCES schedule_patterns(name) ON UPDATE CASCADE ON DELETE SET NULL,
    action_profile    TEXT[]      NOT NULL DEFAULT '{}',
    -- {uuid, planet, position: {x, y, z}}, as in NpcSpec.
    place_where_work  JSONB,
    home              JSONB,
    meal_place        JSONB,
    mandatory_place   JSONB,
    action_timing     JSONB       NOT NULL DEFAULT '{}',
    -- Per-action config blocks of NpcSpec, keyed by action name.
    action_config     JSONB       NOT NULL DEFAULT '{}',
    kc_state          TEXT        NOT NULL DEFAULT 'ready' CHECK (kc_state IN ('ready', 'error')),
    kc_error          TEXT,
    first_spawn_at    TIMESTAMPTZ,
    last_seen_at      TIMESTAMPTZ,
    notes             TEXT,
    created_by        TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((role = 'storekeeper') = (storekeeper_slot IS NOT NULL)),
    CHECK (storekeeper_slot IS NULL OR village_code IS NOT NULL)
);
CREATE UNIQUE INDEX npcs_display_name_uq ON npcs (lower(display_name));
-- At most one storekeeper per slot and village (the first five NPCs of a village).
CREATE UNIQUE INDEX npcs_storekeeper_uq ON npcs (village_code, storekeeper_slot) WHERE storekeeper_slot IS NOT NULL;
CREATE INDEX npcs_village ON npcs (village_code);

-- Where a village's storekeepers work: the cargo_depot object (= universesimulator warehouse id).
CREATE TABLE village_workplaces (
    village_code    TEXT        PRIMARY KEY,
    warehouse_id    UUID        NOT NULL,
    warehouse_code  TEXT,
    scenename       TEXT,
    -- usim: main warehouse from universesimulator; world: nearest replicated cargo_depot; manual: admin.
    source          TEXT        NOT NULL CHECK (source IN ('usim', 'world', 'manual')),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by      TEXT
);

-- Tasks given by hand from the admin panel; they pre-empt the schedule.
CREATE TABLE npc_tasks (
    id            BIGSERIAL   PRIMARY KEY,
    npc_id        UUID        NOT NULL REFERENCES npcs(id) ON DELETE CASCADE,
    kind          TEXT        NOT NULL CHECK (kind IN ('go_to_position', 'run_action', 'wait')),
    params        JSONB       NOT NULL DEFAULT '{}',
    -- Connect the NPC for this task even when its schedule has it off-shift.
    force_online  BOOLEAN     NOT NULL DEFAULT FALSE,
    status        TEXT        NOT NULL DEFAULT 'pending'
                              CHECK (status IN ('pending', 'running', 'done', 'failed', 'timeout', 'cancelled')),
    error         TEXT,
    created_by    TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at    TIMESTAMPTZ,
    finished_at   TIMESTAMPTZ
);
CREATE INDEX npc_tasks_open ON npc_tasks (npc_id, id) WHERE status IN ('pending', 'running');
CREATE INDEX npc_tasks_finished ON npc_tasks (finished_at) WHERE finished_at IS NOT NULL;

-- Key events (spawn, connection, action changes, role, tasks…), kept NPC_EVENTS_RETENTION_DAYS.
CREATE TABLE npc_events (
    id       BIGSERIAL   PRIMARY KEY,
    -- NULL = service-wide event (global switch, schedule import…).
    npc_id   UUID        REFERENCES npcs(id) ON DELETE CASCADE,
    ts       TIMESTAMPTZ NOT NULL DEFAULT now(),
    kind     TEXT        NOT NULL,
    level    TEXT        NOT NULL DEFAULT 'info' CHECK (level IN ('debug', 'info', 'warn', 'error')),
    message  TEXT        NOT NULL,
    data     JSONB
);
CREATE INDEX npc_events_npc_ts ON npc_events (npc_id, ts DESC);
CREATE INDEX npc_events_ts ON npc_events (ts);

-- universesimulator initial schema.
-- Quantities live in inventory (holder type `warehouse`), hierarchy in social, places in
-- persistence, missions in mission and money in economie (ADR 0003). References to those
-- services are opaque UUIDs without foreign keys.

-- ── Federations ──────────────────────────────────────────────────────────────
-- A federation (social `political_entities.type = 'federation'`) governs supply on a planet:
-- it issues money, pays the missions and holds the pricing parameters (ADR 0005, 0008, 0009).
CREATE TABLE federations (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    code                  TEXT        NOT NULL UNIQUE,
    name                  TEXT        NOT NULL,
    political_entity_id   UUID        NOT NULL UNIQUE,
    planet_internal_name  TEXT        NOT NULL,
    -- Persistence object of the planet: its poi_village and cargo_depot children are imported.
    planet_poi_uuid       UUID,
    currency              TEXT        NOT NULL DEFAULT 'credits',
    managed_categories    TEXT[]      NOT NULL DEFAULT '{ore,food,water}',
    -- Monetary policy
    treasury_floor        BIGINT      NOT NULL DEFAULT 100000 CHECK (treasury_floor >= 0),
    mint_batch            BIGINT      NOT NULL DEFAULT 1000000 CHECK (mint_batch > 0),
    mint_ceiling          BIGINT,
    -- Pricing
    elasticity            DOUBLE PRECISION NOT NULL DEFAULT 0.8,
    price_min_factor      DOUBLE PRECISION NOT NULL DEFAULT 0.25,
    price_max_factor      DOUBLE PRECISION NOT NULL DEFAULT 4.0,
    spread                DOUBLE PRECISION NOT NULL DEFAULT 0.1,
    freight_rate_per_tkm  DOUBLE PRECISION NOT NULL DEFAULT 2.0,
    urgency_max           DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    miner_share           DOUBLE PRECISION NOT NULL DEFAULT 0.6,
    leg_base_reward       BIGINT      NOT NULL DEFAULT 50,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT federations_price_bounds CHECK (price_min_factor > 0 AND price_max_factor >= price_min_factor)
);

-- Mirror of the social political hierarchy (refreshed at import and periodically).
CREATE TABLE political_cache (
    id             UUID        PRIMARY KEY,
    type           TEXT        NOT NULL,
    name           TEXT        NOT NULL,
    parent_id      UUID,
    federation_id  UUID        REFERENCES federations(id) ON DELETE SET NULL,
    refreshed_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX political_cache_parent_idx ON political_cache (parent_id);

-- ── Resources ────────────────────────────────────────────────────────────────
CREATE TABLE resources (
    id                   UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    code                 TEXT        NOT NULL UNIQUE,
    name                 TEXT        NOT NULL,
    category             TEXT        NOT NULL,
    unit                 TEXT        NOT NULL DEFAULT 'unit',
    inventory_good_type  TEXT        NOT NULL UNIQUE,
    mass_per_unit_kg     DOUBLE PRECISION NOT NULL DEFAULT 1.0 CHECK (mass_per_unit_kg >= 0),
    base_price           DOUBLE PRECISION NOT NULL CHECK (base_price >= 0),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT resources_category_valid CHECK (category IN ('ore', 'food', 'water', 'fuel', 'material'))
);

-- ── Villages (graph nodes) ───────────────────────────────────────────────────
-- A village is a persistence `poi_village` object (`id` = its UUID) and a commune in social
-- (ADR 0013). Its stock is the sum of its warehouses'; thresholds, flows and mineral sources
-- are set per village. name/x/y/z/population/radius_m/village_type are cached from persistence.
CREATE TABLE villages (
    id                   UUID        PRIMARY KEY,
    code                 TEXT        NOT NULL UNIQUE,
    name                 TEXT        NOT NULL,
    federation_id        UUID        NOT NULL REFERENCES federations(id),
    political_entity_id  UUID        NOT NULL,
    village_type         TEXT,
    population           BIGINT      NOT NULL DEFAULT 0,
    radius_m             DOUBLE PRECISION,
    x                    DOUBLE PRECISION,
    y                    DOUBLE PRECISION,
    z                    DOUBLE PRECISION,
    -- Where deliveries and production go; NULL until a warehouse is attached.
    main_warehouse_id    UUID,
    active               BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX villages_federation_idx ON villages (federation_id) WHERE active;

-- ── Warehouses ───────────────────────────────────────────────────────────────
-- A warehouse is a persistence `cargo_depot` object attached to its village through the
-- `poi_uuid` field of its object_data (ADR 0013). `id` is its UUID and the inventory holder id
-- with holderType `warehouse` (ADR 0004).
CREATE TABLE warehouses (
    id          UUID        PRIMARY KEY,
    village_id  UUID        NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    code        TEXT        NOT NULL UNIQUE,
    name        TEXT        NOT NULL,
    scenename   TEXT,
    x           DOUBLE PRECISION,
    y           DOUBLE PRECISION,
    z           DOUBLE PRECISION,
    active      BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX warehouses_village_idx ON warehouses (village_id);

ALTER TABLE villages ADD CONSTRAINT villages_main_warehouse_fk
    FOREIGN KEY (main_warehouse_id) REFERENCES warehouses(id) ON DELETE SET NULL;

-- ── Route graph between villages (ADR 0006, 0013) ────────────────────────────
CREATE TABLE travel_routes (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    from_village_id  UUID        NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    to_village_id    UUID        NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    distance_m       DOUBLE PRECISION NOT NULL CHECK (distance_m > 0),
    cost             DOUBLE PRECISION NOT NULL CHECK (cost > 0),
    bidirectional    BOOLEAN     NOT NULL DEFAULT TRUE,
    active           BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT travel_routes_not_loop CHECK (from_village_id <> to_village_id),
    CONSTRAINT travel_routes_unique UNIQUE (from_village_id, to_village_id)
);

-- ── Village policies, flows, mineral sources; warehouse capacities ───────────
CREATE TABLE village_policies (
    village_id     UUID        NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    resource_id    UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    min_threshold  BIGINT      NOT NULL CHECK (min_threshold >= 0),
    target_level   BIGINT      NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (village_id, resource_id),
    CONSTRAINT village_policies_levels CHECK (target_level >= min_threshold)
);

-- Physical limits of a warehouse, and the stock credited once when it is first imported.
CREATE TABLE warehouse_capacities (
    warehouse_id           UUID        NOT NULL REFERENCES warehouses(id) ON DELETE CASCADE,
    resource_id            UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    capacity               BIGINT      CHECK (capacity IS NULL OR capacity >= 0),
    initial_stock          BIGINT      NOT NULL DEFAULT 0 CHECK (initial_stock >= 0),
    initial_stock_applied  BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (warehouse_id, resource_id)
);

CREATE TABLE village_flows (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    village_id     UUID        NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    resource_id    UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    -- Positive: production, negative: consumption (units per game hour).
    rate_per_hour  DOUBLE PRECISION NOT NULL,
    carry          DOUBLE PRECISION NOT NULL DEFAULT 0,
    source         TEXT        NOT NULL DEFAULT 'manual',
    label          TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT village_flows_source_valid CHECK (source IN ('population', 'industry', 'manual'))
);

CREATE INDEX village_flows_village_idx ON village_flows (village_id);

CREATE TABLE mineral_sources (
    id                     UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    village_id             UUID        NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    resource_id            UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    -- NULL: inexhaustible.
    remaining_units        BIGINT      CHECK (remaining_units IS NULL OR remaining_units >= 0),
    max_units_per_mission  BIGINT      NOT NULL CHECK (max_units_per_mission > 0),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT mineral_sources_unique UNIQUE (village_id, resource_id)
);

-- Last observed inventory level per warehouse/resource (refreshed every tick and after each
-- write), so list views do not call inventory for every warehouse.
CREATE TABLE stock_levels (
    warehouse_id  UUID        NOT NULL REFERENCES warehouses(id) ON DELETE CASCADE,
    resource_id   UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    quantity      BIGINT      NOT NULL DEFAULT 0,
    held          BIGINT      NOT NULL DEFAULT 0,
    observed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (warehouse_id, resource_id)
);

CREATE TABLE stock_history (
    id            BIGSERIAL   PRIMARY KEY,
    warehouse_id  UUID        NOT NULL REFERENCES warehouses(id) ON DELETE CASCADE,
    resource_id   UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    quantity      BIGINT      NOT NULL,
    held          BIGINT      NOT NULL,
    tick_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX stock_history_lookup_idx ON stock_history (warehouse_id, resource_id, tick_at);

-- ── Prices (ADR 0008) ────────────────────────────────────────────────────────
CREATE TABLE price_index (
    federation_id  UUID        NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    resource_id    UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    supply         BIGINT      NOT NULL,
    demand         BIGINT      NOT NULL,
    unit_price     DOUBLE PRECISION NOT NULL,
    computed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (federation_id, resource_id)
);

CREATE TABLE price_history (
    id             BIGSERIAL   PRIMARY KEY,
    federation_id  UUID        NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    resource_id    UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    supply         BIGINT      NOT NULL,
    demand         BIGINT      NOT NULL,
    unit_price     DOUBLE PRECISION NOT NULL,
    computed_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX price_history_lookup_idx ON price_history (federation_id, resource_id, computed_at);

CREATE TABLE local_prices (
    village_id    UUID        NOT NULL REFERENCES villages(id) ON DELETE CASCADE,
    resource_id   UUID        NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    local_factor  DOUBLE PRECISION NOT NULL,
    buy_price     DOUBLE PRECISION NOT NULL,
    sell_price    DOUBLE PRECISION NOT NULL,
    computed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (village_id, resource_id)
);

-- ── Supply orders (meta-missions) and legs (ADR 0007) ────────────────────────
CREATE TABLE supply_orders (
    id                        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    kind                      TEXT        NOT NULL,
    federation_id             UUID        NOT NULL REFERENCES federations(id),
    resource_id               UUID        NOT NULL REFERENCES resources(id),
    quantity                  BIGINT      NOT NULL CHECK (quantity > 0),
    origin_village_id         UUID        REFERENCES villages(id),
    destination_village_id    UUID        NOT NULL REFERENCES villages(id),
    path                      UUID[]      NOT NULL DEFAULT '{}',
    status                    TEXT        NOT NULL DEFAULT 'planned',
    trigger                   TEXT        NOT NULL,
    total_reward              BIGINT      NOT NULL DEFAULT 0,
    created_by                TEXT,
    last_error                TEXT,
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT supply_orders_kind_valid CHECK (kind IN ('transport', 'mining')),
    CONSTRAINT supply_orders_status_valid CHECK (status IN ('planned', 'in_progress', 'completed', 'failed', 'cancelled')),
    CONSTRAINT supply_orders_trigger_valid CHECK (trigger IN ('auto_shortage', 'auto_mining', 'manual'))
);

CREATE INDEX supply_orders_open_idx ON supply_orders (destination_village_id, resource_id)
    WHERE status IN ('planned', 'in_progress');
CREATE INDEX supply_orders_status_idx ON supply_orders (status, created_at DESC);

CREATE TABLE supply_order_legs (
    id                 UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id           UUID        NOT NULL REFERENCES supply_orders(id) ON DELETE CASCADE,
    seq                INT         NOT NULL CHECK (seq >= 0),
    -- NULL for a mining leg (goods come from the miner).
    from_village_id    UUID        REFERENCES villages(id),
    to_village_id      UUID        NOT NULL REFERENCES villages(id),
    -- Warehouses chosen at publication: where the goods are held, where they are delivered.
    from_warehouse_id  UUID        REFERENCES warehouses(id),
    to_warehouse_id    UUID        REFERENCES warehouses(id),
    quantity           BIGINT      NOT NULL CHECK (quantity > 0),
    distance_m         DOUBLE PRECISION NOT NULL DEFAULT 0,
    reward_amount      BIGINT,
    status             TEXT        NOT NULL DEFAULT 'waiting',
    hold_id            UUID,
    mission_id         UUID,
    attempts           INT         NOT NULL DEFAULT 0,
    last_error         TEXT,
    published_at       TIMESTAMPTZ,
    completed_at       TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT supply_order_legs_status_valid
        CHECK (status IN ('waiting', 'pending', 'held', 'published', 'completed', 'failed', 'cancelled')),
    CONSTRAINT supply_order_legs_seq_unique UNIQUE (order_id, seq)
);

CREATE INDEX supply_order_legs_status_idx ON supply_order_legs (status);
CREATE UNIQUE INDEX supply_order_legs_mission_idx ON supply_order_legs (mission_id) WHERE mission_id IS NOT NULL;

-- ── Off-mission warehouse deliveries (ADR 0012) ──────────────────────────────
CREATE TABLE warehouse_deliveries (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    idempotency_key     TEXT        NOT NULL UNIQUE,
    request_hash        TEXT        NOT NULL,
    warehouse_id        UUID        NOT NULL REFERENCES warehouses(id),
    resource_id         UUID        NOT NULL REFERENCES resources(id),
    requested_quantity  BIGINT      NOT NULL CHECK (requested_quantity > 0),
    accepted_quantity   BIGINT      NOT NULL DEFAULT 0,
    deliverer_type      TEXT        NOT NULL,
    deliverer_id        UUID        NOT NULL,
    status              TEXT        NOT NULL DEFAULT 'pending',
    stock_after         BIGINT,
    metadata            JSONB       NOT NULL DEFAULT '{}',
    last_error          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT warehouse_deliveries_deliverer_valid CHECK (deliverer_type IN ('player', 'npc')),
    CONSTRAINT warehouse_deliveries_status_valid CHECK (status IN ('pending', 'done', 'failed'))
);

-- ── Treasury (ADR 0009) ──────────────────────────────────────────────────────
CREATE TABLE mint_events (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    federation_id  UUID        NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    amount         BIGINT      NOT NULL CHECK (amount > 0),
    reason         TEXT        NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX mint_events_federation_idx ON mint_events (federation_id, created_at DESC);

-- ── Simulation state (single row, ADR 0010) ──────────────────────────────────
CREATE TABLE simulation_state (
    id                     SMALLINT    PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    paused                 BOOLEAN     NOT NULL DEFAULT FALSE,
    last_tick_at           TIMESTAMPTZ,
    tick_count             BIGINT      NOT NULL DEFAULT 0,
    last_tick_duration_ms  BIGINT,
    last_error             TEXT,
    last_political_refresh TIMESTAMPTZ
);

INSERT INTO simulation_state (id) VALUES (1);

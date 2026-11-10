-- Off-mission sales (ADR 0017): goods a player shelved in a warehouse that no mission of theirs counted,
-- bought by the issuer at the market price. A sale is recorded with its price before it is paid
-- (`paid_at` NULL): its key makes it paid once, and a replay resumes a payment that failed half-way.
CREATE TABLE IF NOT EXISTS warehouse_sales (
    idempotency_key TEXT PRIMARY KEY,
    player_id UUID NOT NULL,
    warehouse_id UUID NOT NULL REFERENCES warehouses(id),
    resource_id UUID NOT NULL REFERENCES resources(id),
    quantity BIGINT NOT NULL CHECK (quantity > 0),
    unit_price DOUBLE PRECISION NOT NULL,
    amount BIGINT NOT NULL CHECK (amount >= 0),
    currency TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS warehouse_sales_player ON warehouse_sales (player_id, created_at DESC);

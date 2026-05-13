CREATE TABLE paper_trades (
    id                BIGSERIAL PRIMARY KEY,
    session_id        UUID NOT NULL REFERENCES sessions(id),
    copy_decision_id  BIGINT NOT NULL REFERENCES copy_decisions(id),
    fetched_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    input_mint        BYTEA NOT NULL,
    output_mint       BYTEA NOT NULL,
    in_amount         BIGINT NOT NULL,
    out_amount        BIGINT NOT NULL,
    other_amount_threshold BIGINT NOT NULL,
    price_impact_bps  INTEGER NOT NULL,
    slippage_bps      INTEGER NOT NULL,
    route_labels      TEXT[] NOT NULL DEFAULT '{}',
    quote_latency_ms  INTEGER NOT NULL,
    error             TEXT
);

CREATE INDEX ON paper_trades (session_id, fetched_at);
CREATE INDEX ON paper_trades (copy_decision_id);

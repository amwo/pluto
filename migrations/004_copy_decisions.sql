CREATE TABLE copy_decisions (
    id                BIGSERIAL PRIMARY KEY,
    session_id        UUID NOT NULL REFERENCES sessions(id),
    observed_trade_id BIGINT NOT NULL REFERENCES observed_trades(id),
    decided_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    action            TEXT NOT NULL,
    size_lamports     BIGINT,
    skip_reason       TEXT
);

CREATE INDEX ON copy_decisions (session_id, decided_at);
CREATE INDEX ON copy_decisions (observed_trade_id);
CREATE INDEX ON copy_decisions (action);
CREATE INDEX ON copy_decisions (skip_reason);

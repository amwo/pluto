CREATE TABLE positions (
    id                    BIGSERIAL PRIMARY KEY,
    session_id            UUID NOT NULL REFERENCES sessions(id),
    mint                  BYTEA NOT NULL,
    opened_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    entry_paper_trade_id  BIGINT NOT NULL REFERENCES paper_trades(id),
    entry_in_lamports     BIGINT NOT NULL,
    entry_out_amount      BIGINT NOT NULL,
    entry_price           DOUBLE PRECISION NOT NULL,
    status                TEXT NOT NULL DEFAULT 'open',
    closed_at             TIMESTAMPTZ,
    exit_reason           TEXT,
    exit_paper_trade_id   BIGINT REFERENCES paper_trades(id),
    realized_pnl_lamports BIGINT,
    realized_pnl_pct      DOUBLE PRECISION
);

CREATE INDEX ON positions (session_id, opened_at DESC);
CREATE INDEX ON positions (status, session_id);
CREATE UNIQUE INDEX positions_open_unique ON positions (session_id, mint) WHERE status = 'open';

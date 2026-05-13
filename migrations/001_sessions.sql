CREATE TABLE sessions (
    id         UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at   TIMESTAMPTZ,
    status     TEXT NOT NULL DEFAULT 'running',
    tx_count   BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX ON sessions (created_at DESC);

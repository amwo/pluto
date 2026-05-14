CREATE TABLE latency_samples (
    id           BIGSERIAL PRIMARY KEY,
    session_id   UUID NOT NULL REFERENCES sessions(id),
    kind         TEXT NOT NULL,
    elapsed_ms   INTEGER NOT NULL,
    success      BOOLEAN NOT NULL,
    detail       TEXT,
    sampled_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX ON latency_samples (session_id, kind, sampled_at);

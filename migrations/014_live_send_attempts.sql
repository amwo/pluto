CREATE TABLE live_send_attempts (
    signature       TEXT PRIMARY KEY,
    session_id      UUID NOT NULL REFERENCES sessions(id),
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    bundle_id       TEXT,
    endpoint        TEXT,
    landed          BOOLEAN,
    confirm_error   TEXT
);

CREATE INDEX ON live_send_attempts (session_id, started_at);

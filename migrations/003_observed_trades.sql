CREATE TABLE observed_trades (
    id           BIGSERIAL PRIMARY KEY,
    session_id   UUID NOT NULL REFERENCES sessions(id),
    received_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    slot         BIGINT NOT NULL,
    signature    BYTEA NOT NULL,
    target       BYTEA NOT NULL,
    side         TEXT NOT NULL,
    mint         BYTEA,
    sol_delta_lamports BIGINT NOT NULL,
    token_delta  BIGINT NOT NULL,
    route        TEXT[] NOT NULL DEFAULT '{}',
    jupiter      BOOLEAN NOT NULL,
    pump_swap    BOOLEAN NOT NULL,
    jito_marker  BOOLEAN NOT NULL,
    priority_fee_lamports BIGINT NOT NULL,
    compute_unit_limit INTEGER
);

CREATE INDEX ON observed_trades (session_id, received_at);
CREATE INDEX ON observed_trades (signature);
CREATE INDEX ON observed_trades (mint);
CREATE INDEX ON observed_trades (target);

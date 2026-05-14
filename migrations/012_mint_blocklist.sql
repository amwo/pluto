CREATE TABLE mint_blocklist (
    mint           BYTEA PRIMARY KEY,
    loss_count     INTEGER NOT NULL DEFAULT 0,
    first_loss_at  TIMESTAMPTZ,
    last_loss_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

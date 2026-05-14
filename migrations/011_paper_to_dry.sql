ALTER TABLE paper_trades RENAME TO dry_trades;
ALTER INDEX paper_trades_pkey RENAME TO dry_trades_pkey;
ALTER INDEX paper_trades_session_id_fetched_at_idx RENAME TO dry_trades_session_id_fetched_at_idx;
ALTER INDEX paper_trades_copy_decision_id_idx RENAME TO dry_trades_copy_decision_id_idx;
ALTER SEQUENCE paper_trades_id_seq RENAME TO dry_trades_id_seq;

ALTER TABLE positions RENAME COLUMN entry_paper_trade_id TO entry_dry_trade_id;
ALTER TABLE positions RENAME COLUMN exit_paper_trade_id TO exit_dry_trade_id;

ALTER TABLE sessions ALTER COLUMN mode SET DEFAULT 'dry';

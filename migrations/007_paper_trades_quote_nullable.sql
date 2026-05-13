ALTER TABLE paper_trades
    ALTER COLUMN out_amount DROP NOT NULL,
    ALTER COLUMN other_amount_threshold DROP NOT NULL,
    ALTER COLUMN price_impact_bps DROP NOT NULL,
    ALTER COLUMN route_labels DROP NOT NULL;

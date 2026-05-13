ALTER TABLE positions ADD COLUMN peak_price DOUBLE PRECISION;
UPDATE positions SET peak_price = entry_price WHERE peak_price IS NULL;
ALTER TABLE positions ALTER COLUMN peak_price SET NOT NULL;

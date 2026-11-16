ALTER TABLE transactions ADD COLUMN deleted_at TEXT
    CHECK (deleted_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*');
CREATE INDEX idx_transactions_deleted ON transactions(deleted_at);
CREATE VIEW live_transactions AS SELECT * FROM transactions WHERE deleted_at IS NULL;

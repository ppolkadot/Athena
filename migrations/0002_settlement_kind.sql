-- athena:foreign-keys-off
PRAGMA application_id = 1096044097;

CREATE TABLE transactions_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    date TEXT NOT NULL CHECK (date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'refund', 'settlement')),
    category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    payer_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
    description TEXT,
    split_mode TEXT CHECK (split_mode IN ('even', 'custom')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (kind <> 'settlement' OR (payer_id IS NOT NULL AND category_id IS NULL AND split_mode IS NULL))
);

INSERT INTO transactions_new (id, title, amount_cents, date, kind, category_id, payer_id,
                              description, split_mode, created_at, updated_at)
SELECT id, title, amount_cents, date, kind, category_id, payer_id,
       description, split_mode, created_at, updated_at
FROM transactions;

-- Keep ids of hard-deleted 1.0 rows from being reused.
INSERT INTO sqlite_sequence (name, seq)
SELECT 'transactions_new', seq FROM sqlite_sequence
WHERE name = 'transactions'
  AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'transactions_new');
UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'transactions'), 0))
WHERE name = 'transactions_new';

DROP TABLE transactions;
ALTER TABLE transactions_new RENAME TO transactions;

CREATE INDEX idx_transactions_date ON transactions(date);
CREATE INDEX idx_transactions_category ON transactions(category_id);
CREATE INDEX idx_transactions_payer ON transactions(payer_id);

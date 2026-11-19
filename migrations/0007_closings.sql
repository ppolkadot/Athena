-- Athena 1.4.0 saved monthly closings (contract §14.3). One row per completed month, keyed by its
-- canonical `YYYY-MM`. Figures are historical snapshots; category names resolve live. The largest
-- expense keeps its title and amount after its row is purged (the reference becomes NULL).

CREATE TABLE closings (
    month TEXT PRIMARY KEY CHECK (
        month GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]'
        AND substr(month, 6, 2) BETWEEN '01' AND '12'
    ),
    closed_at TEXT NOT NULL,
    income_cents INTEGER NOT NULL CHECK (income_cents >= 0),
    expenses_cents INTEGER NOT NULL CHECK (expenses_cents >= 0),
    refunds_cents INTEGER NOT NULL CHECK (refunds_cents >= 0),
    result_cents INTEGER NOT NULL,
    entry_count INTEGER NOT NULL CHECK (entry_count >= 0),
    previous_spending_cents INTEGER NOT NULL,
    previous_count INTEGER NOT NULL CHECK (previous_count >= 0),
    top_category_id INTEGER REFERENCES categories(id) ON DELETE RESTRICT,
    top_category_net_cents INTEGER CHECK (top_category_net_cents IS NULL OR top_category_net_cents > 0),
    largest_expense_id INTEGER REFERENCES transactions(id) ON DELETE SET NULL,
    largest_expense_title TEXT,
    largest_expense_cents INTEGER CHECK (largest_expense_cents IS NULL OR largest_expense_cents > 0),
    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version = 1),
    current_fingerprint TEXT NOT NULL CHECK (
        length(current_fingerprint) = 64 AND current_fingerprint NOT GLOB '*[^0-9a-f]*'
    ),
    previous_fingerprint TEXT NOT NULL CHECK (
        length(previous_fingerprint) = 64 AND previous_fingerprint NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (top_category_id IS NULL OR top_category_net_cents IS NOT NULL),
    CHECK ((largest_expense_title IS NULL) = (largest_expense_cents IS NULL)),
    CHECK (largest_expense_id IS NULL OR largest_expense_title IS NOT NULL)
);

CREATE INDEX idx_closings_top_category ON closings(top_category_id);
CREATE INDEX idx_closings_largest_expense ON closings(largest_expense_id);

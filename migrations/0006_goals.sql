-- Athena 1.4.0 savings goals and contributions (contract §14.3). Goal money is independent of the
-- ledger: nothing here references `transactions`.

CREATE TABLE goals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    target_cents INTEGER NOT NULL CHECK (target_cents > 0),
    target_date TEXT CHECK (
        target_date IS NULL
        OR target_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
    ),
    frequency TEXT NOT NULL CHECK (frequency IN ('daily', 'weekly', 'monthly')),
    attribution_id INTEGER REFERENCES people(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('active', 'paused', 'completed', 'archived')),
    pre_archive_status TEXT CHECK (
        pre_archive_status IS NULL OR pre_archive_status IN ('active', 'paused', 'completed')
    ),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((status = 'archived') = (pre_archive_status IS NOT NULL))
);

CREATE TABLE goal_contributions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    goal_id INTEGER NOT NULL REFERENCES goals(id) ON DELETE RESTRICT,
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    date TEXT NOT NULL CHECK (date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    note TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_goals_attribution ON goals(attribution_id);
CREATE INDEX idx_goal_contributions_goal_date ON goal_contributions(goal_id, date, id);

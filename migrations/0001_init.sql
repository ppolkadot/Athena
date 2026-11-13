-- Athena 1.0.0 schema. Money is integer cents. Timestamps are UTC RFC 3339. Dates are ISO local dates.

CREATE TABLE profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    currency TEXT NOT NULL DEFAULT 'BRL' CHECK (currency = 'BRL'),
    start_balance_cents INTEGER NOT NULL DEFAULT 0,
    theme TEXT NOT NULL DEFAULT 'dark' CHECK (theme IN ('dark', 'light')),
    language TEXT NOT NULL DEFAULT 'en' CHECK (language IN ('en', 'pt-BR')),
    onboarding_complete INTEGER NOT NULL DEFAULT 0 CHECK (onboarding_complete IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('owner', 'partner', 'other')),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL,
    CHECK (kind <> 'owner' OR archived = 0)
);
CREATE UNIQUE INDEX idx_people_one_owner ON people(kind) WHERE kind = 'owner';

CREATE TABLE categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK (length(trim(name)) > 0),
    applies_to TEXT NOT NULL DEFAULT 'expense' CHECK (applies_to IN ('expense', 'income', 'both')),
    color TEXT NOT NULL DEFAULT '#8F98A8'
        CHECK (color GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
    icon TEXT,
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE TABLE transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    date TEXT NOT NULL CHECK (date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'refund')),
    category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    payer_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
    description TEXT,
    split_mode TEXT CHECK (split_mode IN ('even', 'custom')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE allocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE RESTRICT,
    amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
    UNIQUE (transaction_id, person_id)
);

CREATE INDEX idx_transactions_date ON transactions(date);
CREATE INDEX idx_transactions_category ON transactions(category_id);
CREATE INDEX idx_transactions_payer ON transactions(payer_id);
CREATE INDEX idx_allocations_person ON allocations(person_id);

-- Seeded categories. Rows are ordinary: renamable and archivable. Names are not translated in storage.
INSERT INTO categories (name, applies_to, color, is_default, created_at) VALUES
    ('Food',            'expense', '#FF8A65', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Transport',       'expense', '#4FC3F7', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Housing',         'expense', '#BA68C8', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Education',       'expense', '#64B5F6', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Health',          'expense', '#81C784', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Entertainment',   'expense', '#FFD54F', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Shopping',        'expense', '#F06292', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Subscriptions',   'expense', '#9575CD', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Bills & utilities', 'expense', '#90A4AE', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Other',           'both',    '#8F98A8', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Salary',          'income',  '#3DDC97', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('Other income',    'income',  '#26A69A', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- Athena 1.2.0 entry templates (contract §14.1). Settlements are not templatable.

CREATE TABLE templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'refund')),
    category_id INTEGER REFERENCES categories(id) ON DELETE RESTRICT,
    payer_id INTEGER REFERENCES people(id) ON DELETE RESTRICT,
    responsibility TEXT NOT NULL CHECK (responsibility IN ('none', 'person', 'even', 'custom')),
    description TEXT,
    created_at TEXT NOT NULL,
    last_used_at TEXT
);

CREATE TABLE template_allocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id INTEGER NOT NULL REFERENCES templates(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE RESTRICT,
    amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
    UNIQUE (template_id, person_id)
);

CREATE INDEX idx_templates_category ON templates(category_id);
CREATE INDEX idx_templates_payer ON templates(payer_id);
CREATE INDEX idx_template_allocations_person ON template_allocations(person_id);

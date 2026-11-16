//! Shared test helpers. Compiled only for tests. Database-backed helpers (`seed_people`,
//! `seed_month`, `seeded_db`, ...) are added as their repositories land.

#![allow(clippy::must_use_candidate, clippy::missing_panics_doc)]

use chrono::NaiveDate;

use crate::domain::money::Money;
use crate::domain::person::{Person, PersonKind};
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::transaction::{Allocation, SplitMode, Transaction, TransactionType, TxDraft};

pub fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

pub fn today() -> NaiveDate {
    date(2026, 11, 11)
}

pub fn tx_on(y: i32, m: u32, d: u32) -> Transaction {
    Transaction {
        id: 0,
        title: "Test".into(),
        amount: Money::from_cents(100),
        date: date(y, m, d),
        kind: TransactionType::Expense,
        category_id: None,
        payer_id: None,
        description: None,
        split_mode: None,
        allocations: vec![],
    }
}

/// Title `Test`, amount = sum of allocations (R$ 1,00 when there are none, since a transaction
/// amount must be positive), dated [`today`], an expense with no payer.
pub fn draft_with(allocations: Vec<Allocation>) -> TxDraft {
    let sum: i64 = allocations.iter().map(|a| a.amount.cents()).sum();
    let amount = Money::from_cents(if sum > 0 { sum } else { 100 });
    let split_mode = if allocations.len() > 1 {
        Some(SplitMode::Even)
    } else {
        None
    };
    TxDraft {
        title: "Test".into(),
        amount,
        date: today(),
        kind: TransactionType::Expense,
        category_id: None,
        payer_id: None,
        description: None,
        allocations,
        split_mode,
    }
}

pub fn person(id: i64, name: &str, kind: PersonKind) -> Person {
    Person {
        id,
        name: name.into(),
        kind,
        archived: false,
        created_at: "2026-11-11T00:00:00Z".into(),
    }
}

pub fn two_people() -> Vec<Person> {
    vec![
        person(1, "A3", PersonKind::Owner),
        person(2, "Ana", PersonKind::Partner),
    ]
}

pub fn three_people() -> Vec<Person> {
    vec![
        person(1, "A3", PersonKind::Owner),
        person(2, "Ana", PersonKind::Partner),
        person(3, "Bia", PersonKind::Other),
    ]
}

pub fn profile() -> Profile {
    Profile {
        start_balance: Money::from_cents(100_000),
        theme: Theme::Dark,
        language: Language::En,
        onboarding_complete: true,
    }
}

/// The owner `A3` and partner `Ana`, created once. Returns `(owner_id, partner_id)`.
pub fn seed_people(conn: &rusqlite::Connection) -> (i64, i64) {
    use crate::infrastructure::db::people_repo;
    let existing = people_repo::list(conn, true).unwrap();
    let owner = match existing.iter().find(|p| p.kind == PersonKind::Owner) {
        Some(p) => p.id,
        None => people_repo::insert(conn, "A3", PersonKind::Owner).unwrap(),
    };
    let partner = match existing.iter().find(|p| p.kind == PersonKind::Partner) {
        Some(p) => p.id,
        None => people_repo::insert(conn, "Ana", PersonKind::Partner).unwrap(),
    };
    (owner, partner)
}

pub fn seeded_category(conn: &rusqlite::Connection, name: &str) -> i64 {
    use crate::infrastructure::db::categories_repo;
    categories_repo::list(conn, true)
        .unwrap()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap()
        .id
}

fn alloc(person_id: i64, cents: i64) -> Allocation {
    Allocation {
        person_id,
        amount: Money::from_cents(cents),
    }
}

fn create(db: &crate::infrastructure::db::Db, draft: &TxDraft) -> i64 {
    crate::infrastructure::db::transactions_repo::create(db.conn(), draft).unwrap()
}

pub fn seed_one_expense(db: &crate::infrastructure::db::Db) -> i64 {
    let (owner, _) = seed_people(db.conn());
    create(db, &draft_with(vec![alloc(owner, 100)]))
}

pub fn seed_one_shared(db: &crate::infrastructure::db::Db) -> i64 {
    let (owner, partner) = seed_people(db.conn());
    create(
        db,
        &draft_with(vec![alloc(owner, 100), alloc(partner, 100)]),
    )
}

/// November 2026: income 300,00; expenses 150,00 (Rent 100,00 Housing, Market 50,00 Food shared);
/// refund 20,00. Count 4; in cents: expenses 15000, refunds 2000, net 17000, spending 13000.
/// Top category `Housing`, largest expense `Rent`.
pub fn seed_month(db: &crate::infrastructure::db::Db) {
    let (owner, partner) = seed_people(db.conn());
    let housing = seeded_category(db.conn(), "Housing");
    let food = seeded_category(db.conn(), "Food");
    let mut rent = draft_with(vec![alloc(owner, 10_000)]);
    rent.title = "Rent".into();
    rent.category_id = Some(housing);
    rent.payer_id = Some(owner);
    rent.date = date(2026, 11, 1);
    create(db, &rent);
    let mut market = draft_with(vec![alloc(owner, 2_500), alloc(partner, 2_500)]);
    market.title = "Market".into();
    market.category_id = Some(food);
    market.payer_id = Some(owner);
    market.date = date(2026, 11, 5);
    create(db, &market);
    let mut refund = draft_with(vec![alloc(owner, 2_000)]);
    refund.title = "Refund".into();
    refund.kind = TransactionType::Refund;
    refund.payer_id = Some(owner);
    refund.date = date(2026, 11, 8);
    create(db, &refund);
    let mut salary = draft_with(vec![]);
    salary.title = "Salary".into();
    salary.amount = Money::from_cents(30_000);
    salary.kind = TransactionType::Income;
    salary.date = date(2026, 11, 5);
    create(db, &salary);
}

/// One shared 200,00 expense: owner 120,00, partner 80,00.
pub fn seed_shared(db: &crate::infrastructure::db::Db) {
    let (owner, partner) = seed_people(db.conn());
    let mut draft = draft_with(vec![alloc(owner, 12_000), alloc(partner, 8_000)]);
    draft.split_mode = Some(SplitMode::Custom);
    create(db, &draft);
}

pub fn seed_mixed(db: &crate::infrastructure::db::Db) {
    seed_month(db);
}

/// November 2026 with one row of each kind: expense 100,00 (Housing, A3), income 300,00,
/// refund 20,00 (A3), and settlement 80,00 (Ana -> A3).
pub fn seed_all_kinds(db: &crate::infrastructure::db::Db) -> (i64, i64) {
    let (owner, partner) = seed_people(db.conn());
    let housing = seeded_category(db.conn(), "Housing");

    let mut expense = draft_with(vec![alloc(owner, 10_000)]);
    expense.title = "Rent".into();
    expense.category_id = Some(housing);
    expense.payer_id = Some(owner);
    expense.date = date(2026, 11, 1);
    create(db, &expense);

    let mut income = draft_with(vec![]);
    income.title = "Salary".into();
    income.amount = Money::from_cents(30_000);
    income.kind = TransactionType::Income;
    income.date = date(2026, 11, 5);
    create(db, &income);

    let mut refund = draft_with(vec![alloc(owner, 2_000)]);
    refund.title = "Refund".into();
    refund.kind = TransactionType::Refund;
    refund.payer_id = Some(owner);
    refund.date = date(2026, 11, 8);
    create(db, &refund);

    let settlement = TxDraft {
        title: "Ana -> A3".into(),
        amount: Money::from_cents(8_000),
        date: date(2026, 11, 10),
        kind: TransactionType::Settlement,
        category_id: None,
        payer_id: Some(partner),
        description: None,
        allocations: vec![alloc(owner, 8_000)],
        split_mode: None,
    };
    create(db, &settlement);
    (owner, partner)
}

/// Three transactions ordered by date then id: a 200,00 shared A3/Ana split, a 1234,56 expense
/// and a 50,00 refund.
pub fn seeded_db() -> crate::infrastructure::db::Db {
    let db = crate::infrastructure::db::Db::open_in_memory().unwrap();
    let (owner, partner) = seed_people(db.conn());
    let mut shared = draft_with(vec![alloc(owner, 10_000), alloc(partner, 10_000)]);
    shared.date = date(2026, 11, 10);
    create(&db, &shared);
    let mut big = draft_with(vec![alloc(owner, 123_456)]);
    big.date = date(2026, 11, 11);
    create(&db, &big);
    let mut refund = draft_with(vec![alloc(owner, 5_000)]);
    refund.kind = TransactionType::Refund;
    refund.date = date(2026, 11, 11);
    create(&db, &refund);
    db
}

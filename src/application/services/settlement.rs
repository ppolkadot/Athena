//! Settlement balance calculation for the selected month.

use rusqlite::Connection;

use crate::domain::date::YearMonth;
use crate::domain::settlement::{self, SettlementView};
use crate::infrastructure::db::settlement_repo;
use crate::shared::error::Result;

/// Compute cumulative settlement balances and suggested transfers as of the last day of `month`.
///
/// # Errors
/// Database or overflow validation errors.
pub fn for_month(conn: &Connection, month: YearMonth) -> Result<SettlementView> {
    let as_of = month.last_day();
    let inputs = settlement_repo::list_inputs(conn, as_of)?;
    let view = settlement::calculate(as_of, &inputs)?;
    if view.excluded.invalid > 0 {
        tracing::warn!(
            invalid = view.excluded.invalid,
            as_of = %view.as_of,
            "excluded invalid rows from settlement calculation"
        );
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::settlement::SuggestedTransfer;
    use crate::domain::transaction::{Allocation, TransactionType};
    use crate::infrastructure::db::{Db, transactions_repo};
    use crate::shared::test_fixtures::{date, draft_with, seed_people};

    #[test]
    fn settlement_is_cumulative_as_of_month_end() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());

        let mut expense = draft_with(vec![
            Allocation {
                person_id: owner,
                amount: Money::from_cents(5_000),
            },
            Allocation {
                person_id: partner,
                amount: Money::from_cents(5_000),
            },
        ]);
        expense.title = "Groceries".into();
        expense.payer_id = Some(owner);
        expense.date = date(2026, 11, 10);
        transactions_repo::create(db.conn(), &expense).unwrap();

        let mut payback = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(5_000),
        }]);
        payback.title = "Payback".into();
        payback.kind = TransactionType::Settlement;
        payback.payer_id = Some(partner);
        payback.date = date(2026, 12, 5);
        transactions_repo::create(db.conn(), &payback).unwrap();

        let nov = for_month(db.conn(), YearMonth::new(2026, 11).unwrap()).unwrap();
        assert_eq!(
            nov.transfers,
            vec![SuggestedTransfer {
                from: partner,
                to: owner,
                amount: Money::from_cents(5_000),
            }]
        );

        let dec = for_month(db.conn(), YearMonth::new(2026, 12).unwrap()).unwrap();
        assert_eq!(dec.balances.len(), 2);
        assert!(dec.balances.iter().all(|b| b.balance.is_zero()));
        assert!(dec.transfers.is_empty());
    }
}

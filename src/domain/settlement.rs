//! Pure settlement calculation: cumulative net balances and greedy transfers.
//! No I/O or UI formatting lives in this module.

use std::collections::{BTreeMap, HashSet};

use chrono::NaiveDate;

use super::money::Money;
use super::transaction::{Allocation, TransactionType};
use crate::shared::error::{AppError, Result};

/// Minimal transaction projection needed to compute cumulative settlement balances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementInput {
    pub kind: TransactionType,
    pub amount: Money,
    pub payer_id: Option<i64>,
    pub allocations: Vec<Allocation>,
}

/// Counts of live non-income rows up to `as_of` excluded from settlement math.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExclusionCounts {
    pub unassigned: usize,
    pub unknown_payer: usize,
    pub invalid: usize,
}

impl ExclusionCounts {
    #[must_use]
    pub fn any(self) -> bool {
        self.unassigned > 0 || self.unknown_payer > 0 || self.invalid > 0
    }
}

/// One participant's cumulative paid, owed and net balance (`paid - owed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersonBalance {
    pub person_id: i64,
    pub paid: Money,
    pub owed: Money,
    pub balance: Money,
}

/// Deterministic transfer needed to settle debts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuggestedTransfer {
    pub from: i64,
    pub to: i64,
    pub amount: Money,
}

/// Cumulative settlement result as of an inclusive cut-off date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementView {
    pub as_of: NaiveDate,
    pub balances: Vec<PersonBalance>,
    pub transfers: Vec<SuggestedTransfer>,
    pub excluded: ExclusionCounts,
}

fn overflow_err() -> AppError {
    AppError::Validation("settlement calculation overflow".into())
}

fn add_deltas(
    totals: &mut BTreeMap<i64, (i64, i64)>,
    person_id: i64,
    paid_delta: i64,
    owed_delta: i64,
) -> Result<()> {
    let entry = totals.entry(person_id).or_insert((0, 0));
    entry.0 = entry.0.checked_add(paid_delta).ok_or_else(overflow_err)?;
    entry.1 = entry.1.checked_add(owed_delta).ok_or_else(overflow_err)?;
    Ok(())
}

fn expense_or_refund_valid(amount: Money, allocations: &[Allocation]) -> bool {
    if amount.cents() <= 0 {
        return false;
    }
    let mut seen = HashSet::with_capacity(allocations.len());
    let mut sum = 0i64;
    for alloc in allocations {
        if alloc.amount.cents() < 0 || !seen.insert(alloc.person_id) {
            return false;
        }
        let Some(next) = sum.checked_add(alloc.amount.cents()) else {
            return false;
        };
        sum = next;
    }
    sum == amount.cents()
}

fn match_transfers(balances: &[PersonBalance]) -> Result<Vec<SuggestedTransfer>> {
    let mut creditors: Vec<(i64, i64)> = balances
        .iter()
        .filter(|b| b.balance.cents() > 0)
        .map(|b| (b.person_id, b.balance.cents()))
        .collect();
    let mut debtors: Vec<(i64, i64)> = balances
        .iter()
        .filter(|b| b.balance.cents() < 0)
        .map(|b| {
            let debt = 0i64
                .checked_sub(b.balance.cents())
                .ok_or_else(overflow_err)?;
            Ok((b.person_id, debt))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut transfers = Vec::new();
    while let Some((c_idx, _)) = creditors
        .iter()
        .enumerate()
        .filter(|(_, (_, amt))| *amt > 0)
        .max_by(|(_, a), (_, b)| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
    {
        let Some((d_idx, _)) = debtors
            .iter()
            .enumerate()
            .filter(|(_, (_, amt))| *amt > 0)
            .max_by(|(_, a), (_, b)| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        else {
            break;
        };
        let amount = creditors[c_idx].1.min(debtors[d_idx].1);
        transfers.push(SuggestedTransfer {
            from: debtors[d_idx].0,
            to: creditors[c_idx].0,
            amount: Money::from_cents(amount),
        });
        creditors[c_idx].1 -= amount;
        debtors[d_idx].1 -= amount;
    }
    Ok(transfers)
}

/// Compute cumulative balances and greedy deterministic transfers for all rows up to `as_of`.
///
/// # Errors
/// Returns [`AppError::Validation`] if any participant total, balance or debt negation overflows
/// signed 64-bit cents.
pub fn calculate(as_of: NaiveDate, inputs: &[SettlementInput]) -> Result<SettlementView> {
    let mut totals: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
    let mut excluded = ExclusionCounts::default();

    for row in inputs {
        match row.kind {
            TransactionType::Income => {}
            TransactionType::Expense | TransactionType::Refund => {
                if row.allocations.is_empty() {
                    excluded.unassigned += 1;
                    continue;
                }
                let Some(payer_id) = row.payer_id else {
                    excluded.unknown_payer += 1;
                    continue;
                };
                if !expense_or_refund_valid(row.amount, &row.allocations) {
                    excluded.invalid += 1;
                    continue;
                }
                let sign = if row.kind == TransactionType::Refund {
                    -1i64
                } else {
                    1i64
                };
                let paid_delta = row
                    .amount
                    .cents()
                    .checked_mul(sign)
                    .ok_or_else(overflow_err)?;
                add_deltas(&mut totals, payer_id, paid_delta, 0)?;
                for alloc in &row.allocations {
                    let owed_delta = alloc
                        .amount
                        .cents()
                        .checked_mul(sign)
                        .ok_or_else(overflow_err)?;
                    add_deltas(&mut totals, alloc.person_id, 0, owed_delta)?;
                }
            }
            TransactionType::Settlement => {
                let Some(from) = row.payer_id else {
                    excluded.invalid += 1;
                    continue;
                };
                let [recipient] = row.allocations.as_slice() else {
                    excluded.invalid += 1;
                    continue;
                };
                if row.amount.cents() <= 0
                    || recipient.person_id == from
                    || recipient.amount != row.amount
                {
                    excluded.invalid += 1;
                    continue;
                }
                add_deltas(&mut totals, from, row.amount.cents(), 0)?;
                add_deltas(&mut totals, recipient.person_id, 0, row.amount.cents())?;
            }
        }
    }

    let mut balances: Vec<PersonBalance> = totals
        .into_iter()
        .filter(|(_, (paid, owed))| *paid != 0 || *owed != 0)
        .map(|(person_id, (paid, owed))| {
            let balance = paid.checked_sub(owed).ok_or_else(overflow_err)?;
            Ok(PersonBalance {
                person_id,
                paid: Money::from_cents(paid),
                owed: Money::from_cents(owed),
                balance: Money::from_cents(balance),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    balances.sort_by(|a, b| {
        b.balance
            .cmp(&a.balance)
            .then_with(|| a.person_id.cmp(&b.person_id))
    });

    let transfers = match_transfers(&balances)?;

    Ok(SettlementView {
        as_of,
        balances,
        transfers,
        excluded,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::domain::transaction::split_evenly;

    fn d() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()
    }

    fn alloc(person_id: i64, cents: i64) -> Allocation {
        Allocation {
            person_id,
            amount: Money::from_cents(cents),
        }
    }

    #[test]
    fn expense_refund_settlement_directions() {
        // Expense 100 paid by 1, shared evenly by 1 and 2 -> 1:+50, 2:-50.
        let expense = SettlementInput {
            kind: TransactionType::Expense,
            amount: Money::from_cents(10_000),
            payer_id: Some(1),
            allocations: vec![alloc(1, 5_000), alloc(2, 5_000)],
        };
        let after_exp = calculate(d(), std::slice::from_ref(&expense)).unwrap();
        assert_eq!(
            after_exp.balances,
            vec![
                PersonBalance {
                    person_id: 1,
                    paid: Money::from_cents(10_000),
                    owed: Money::from_cents(5_000),
                    balance: Money::from_cents(5_000),
                },
                PersonBalance {
                    person_id: 2,
                    paid: Money::ZERO,
                    owed: Money::from_cents(5_000),
                    balance: Money::from_cents(-5_000),
                },
            ]
        );
        assert_eq!(
            after_exp.transfers,
            vec![SuggestedTransfer {
                from: 2,
                to: 1,
                amount: Money::from_cents(5_000),
            }]
        );

        // Refund 20 to payer 1, reducing owed by 10 each -> 1:+40, 2:-40.
        let refund = SettlementInput {
            kind: TransactionType::Refund,
            amount: Money::from_cents(2_000),
            payer_id: Some(1),
            allocations: vec![alloc(1, 1_000), alloc(2, 1_000)],
        };
        let after_refund = calculate(d(), &[expense.clone(), refund.clone()]).unwrap();
        assert_eq!(
            after_refund.balances,
            vec![
                PersonBalance {
                    person_id: 1,
                    paid: Money::from_cents(8_000),
                    owed: Money::from_cents(4_000),
                    balance: Money::from_cents(4_000),
                },
                PersonBalance {
                    person_id: 2,
                    paid: Money::ZERO,
                    owed: Money::from_cents(4_000),
                    balance: Money::from_cents(-4_000),
                },
            ]
        );

        // Settlement 40 from 2 to 1 zeroes both balances while keeping both as participants.
        let settlement = SettlementInput {
            kind: TransactionType::Settlement,
            amount: Money::from_cents(4_000),
            payer_id: Some(2),
            allocations: vec![alloc(1, 4_000)],
        };
        let settled = calculate(d(), &[expense, refund, settlement]).unwrap();
        assert_eq!(settled.balances.len(), 2);
        assert!(settled.balances.iter().all(|b| b.balance.is_zero()));
        assert!(settled.transfers.is_empty());
    }

    #[test]
    fn exclusion_reasons_are_ordered_and_counted() {
        let inputs = vec![
            // Unassigned with payer set -> Unassigned
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(1_000),
                payer_id: Some(1),
                allocations: vec![],
            },
            // Both empty allocations and missing payer -> Unassigned wins first!
            SettlementInput {
                kind: TransactionType::Refund,
                amount: Money::from_cents(1_000),
                payer_id: None,
                allocations: vec![],
            },
            // Non-empty allocations, missing payer -> UnknownPayer
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(2_000),
                payer_id: None,
                allocations: vec![alloc(1, 1_000), alloc(2, 1_000)],
            },
            // Sum mismatch with payer present -> Invalid
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(2_000),
                payer_id: Some(1),
                allocations: vec![alloc(1, 500)],
            },
        ];
        let view = calculate(d(), &inputs).unwrap();
        assert_eq!(
            view.excluded,
            ExclusionCounts {
                unassigned: 2,
                unknown_payer: 1,
                invalid: 1,
            }
        );
        assert!(view.balances.is_empty());
        assert!(view.transfers.is_empty());
    }

    #[test]
    fn income_is_ignored_and_not_counted_as_excluded() {
        let inputs = vec![SettlementInput {
            kind: TransactionType::Income,
            amount: Money::from_cents(50_000),
            payer_id: None,
            allocations: vec![],
        }];
        let view = calculate(d(), &inputs).unwrap();
        assert_eq!(view.excluded, ExclusionCounts::default());
        assert!(view.balances.is_empty());
        assert!(view.transfers.is_empty());
    }

    #[test]
    fn archived_participants_are_included() {
        // The pure domain function has no archived flag and includes every non-zero participant ID.
        let inputs = vec![SettlementInput {
            kind: TransactionType::Expense,
            amount: Money::from_cents(3_000),
            payer_id: Some(1),
            allocations: vec![alloc(99, 3_000)],
        }];
        let view = calculate(d(), &inputs).unwrap();
        assert_eq!(
            view.balances,
            vec![
                PersonBalance {
                    person_id: 1,
                    paid: Money::from_cents(3_000),
                    owed: Money::ZERO,
                    balance: Money::from_cents(3_000),
                },
                PersonBalance {
                    person_id: 99,
                    paid: Money::ZERO,
                    owed: Money::from_cents(3_000),
                    balance: Money::from_cents(-3_000),
                },
            ]
        );
    }

    #[test]
    fn transfers_are_greedy_deterministic_and_exact() {
        // Balances:
        // 1: +50 (40+10), 2: +50 -> tie on creditors broken by lowest id (1 first)
        // 3: -60, 4: -40 -> largest debtor is 3 (-60)
        // Expected greedy sequence:
        // Step 1: 3 -> 1: 50 (1 becomes 0, 3 becomes -10)
        // Step 2: 4 -> 2: 40 (4 becomes 0, 2 becomes +10; max debtor was 4:-40 vs 3:-10)
        // Step 3: 3 -> 2: 10 (both 0)
        let inputs = vec![
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(5_000),
                payer_id: Some(1),
                allocations: vec![alloc(3, 5_000)],
            },
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(5_000),
                payer_id: Some(2),
                allocations: vec![alloc(3, 1_000), alloc(4, 4_000)],
            },
        ];
        let view = calculate(d(), &inputs).unwrap();
        assert_eq!(
            view.transfers,
            vec![
                SuggestedTransfer {
                    from: 3,
                    to: 1,
                    amount: Money::from_cents(5_000),
                },
                SuggestedTransfer {
                    from: 4,
                    to: 2,
                    amount: Money::from_cents(4_000),
                },
                SuggestedTransfer {
                    from: 3,
                    to: 2,
                    amount: Money::from_cents(1_000),
                },
            ]
        );
        // Verify balance sort: balance desc, then person_id asc (1 before 2 at +5000).
        let ids: Vec<i64> = view.balances.iter().map(|b| b.person_id).collect();
        assert_eq!(ids, vec![1, 2, 4, 3]);
    }

    #[test]
    fn checked_overflow_is_an_error() {
        // Paid accumulation overflow:
        let paid_overflow = vec![
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(i64::MAX),
                payer_id: Some(1),
                allocations: vec![alloc(2, i64::MAX)],
            },
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(1),
                payer_id: Some(1),
                allocations: vec![alloc(2, 1)],
            },
        ];
        assert!(matches!(
            calculate(d(), &paid_overflow),
            Err(AppError::Validation(_))
        ));

        // Balance = paid - owed overflow (i64::MAX - (-1)):
        let balance_overflow = vec![
            SettlementInput {
                kind: TransactionType::Expense,
                amount: Money::from_cents(i64::MAX),
                payer_id: Some(1),
                allocations: vec![alloc(2, i64::MAX)],
            },
            SettlementInput {
                kind: TransactionType::Refund,
                amount: Money::from_cents(1),
                payer_id: Some(2),
                allocations: vec![alloc(1, 1)],
            },
        ];
        assert!(matches!(
            calculate(d(), &balance_overflow),
            Err(AppError::Validation(_))
        ));
    }

    fn arb_input() -> impl Strategy<Value = SettlementInput> {
        (1i64..=6, 1i64..=50_000, 0u8..4).prop_flat_map(|(payer, cents, kind_idx)| {
            let kind = match kind_idx {
                0 => TransactionType::Income,
                1 => TransactionType::Refund,
                2 => TransactionType::Settlement,
                _ => TransactionType::Expense,
            };
            let recipient = if payer == 6 { 1 } else { payer + 1 };
            (1usize..=4).prop_map(move |split_n| match kind {
                TransactionType::Income => SettlementInput {
                    kind,
                    amount: Money::from_cents(cents),
                    payer_id: Some(payer),
                    allocations: vec![],
                },
                TransactionType::Settlement => SettlementInput {
                    kind,
                    amount: Money::from_cents(cents),
                    payer_id: Some(payer),
                    allocations: vec![alloc(recipient, cents)],
                },
                TransactionType::Expense | TransactionType::Refund => {
                    let weights = vec![1u32; split_n];
                    let shares = split_evenly(Money::from_cents(cents), &weights).unwrap();
                    let allocations = shares
                        .into_iter()
                        .enumerate()
                        .map(|(i, amount)| Allocation {
                            person_id: i64::try_from(i).unwrap() + 1,
                            amount,
                        })
                        .collect();
                    SettlementInput {
                        kind,
                        amount: Money::from_cents(cents),
                        payer_id: Some(payer),
                        allocations,
                    }
                }
            })
        })
    }

    proptest! {
        #[test]
        fn balances_sum_to_zero(inputs in proptest::collection::vec(arb_input(), 0..30)) {
            let view = calculate(d(), &inputs).unwrap();
            let sum: i128 = view
                .balances
                .iter()
                .map(|b| i128::from(b.balance.cents()))
                .sum();
            prop_assert_eq!(sum, 0);
            prop_assert!(view.transfers.len() <= view.balances.len().saturating_sub(1));

            let mut net: BTreeMap<i64, i64> = view
                .balances
                .iter()
                .map(|b| (b.person_id, b.balance.cents()))
                .collect();
            for transfer in &view.transfers {
                prop_assert!(transfer.amount.cents() > 0);
                *net.get_mut(&transfer.from).unwrap() += transfer.amount.cents();
                *net.get_mut(&transfer.to).unwrap() -= transfer.amount.cents();
            }
            prop_assert!(net.values().all(|v| *v == 0));
        }
    }
}

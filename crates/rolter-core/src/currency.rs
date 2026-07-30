//! Currency codes and conversion for cost accounting.
//!
//! A currency code is an **open string**, never an enum. Today that means
//! ISO-4217 (`USD`, `EUR`, `GBP`); tomorrow it may mean a crypto or custom
//! settlement unit (`BTC`, `USDC`, internal credits). Adding one must cost a
//! rate-table entry and nothing else — no code change anywhere in the stack.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The settlement currency spend accumulates in when none is configured.
pub const DEFAULT_BASE_CURRENCY: &str = "USD";

pub(crate) fn default_base_currency() -> String {
    DEFAULT_BASE_CURRENCY.to_string()
}

/// Normalize a currency code for lookup: trimmed and upper-cased, so `usd`,
/// `USD ` and `Usd` are the same currency. Codes are compared this way
/// everywhere; the original spelling is preserved for display.
pub fn normalize_code(code: &str) -> String {
    code.trim().to_ascii_uppercase()
}

/// Convert an amount between currency codes.
///
/// Implementations are free to source rates however they like — an operator's
/// static table, a slow-cadence feed, a ledger — but they must be honest about
/// what they do not know: an unknown pair returns `None` rather than a guess,
/// so the caller can fail closed instead of silently charging the wrong number.
pub trait CurrencyConverter: Send + Sync {
    /// Amount expressed in `to`, or `None` when the pair is not convertible.
    fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64>;
}

/// Operator-configured currency settings: the base everything settles in, plus
/// a static rate table.
///
/// Static rates are the only offline-safe source, and rolter must run
/// air-gapped, so this is the default and every other source degrades to it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurrencyConfig {
    /// currency all budgets and accumulated spend are denominated in
    #[serde(default = "default_base_currency")]
    pub base: String,
    /// how many units of `base` one unit of the keyed currency is worth
    /// (`EUR = 1.09` means one euro costs 1.09 base units when base is USD).
    /// The base itself is implicitly 1.0 and need not be listed
    #[serde(default)]
    pub rates: HashMap<String, f64>,
}

impl Default for CurrencyConfig {
    fn default() -> Self {
        Self {
            base: default_base_currency(),
            rates: HashMap::new(),
        }
    }
}

impl CurrencyConfig {
    /// Base currency, normalized for comparison.
    pub fn base_code(&self) -> String {
        normalize_code(&self.base)
    }

    /// Rate for `code` in units of base, or `None` when the table has no entry.
    /// The base currency is always 1.0 without needing a row.
    pub fn rate(&self, code: &str) -> Option<f64> {
        let code = normalize_code(code);
        if code == self.base_code() {
            return Some(1.0);
        }
        self.rates
            .iter()
            .find(|(k, _)| normalize_code(k) == code)
            .map(|(_, rate)| *rate)
    }

    /// Problems that make this table unusable, in the shape
    /// [`crate::GatewayConfig::validate`] collects.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.base.trim().is_empty() {
            problems.push("currency.base must not be empty".to_string());
        }
        for (code, rate) in &self.rates {
            if code.trim().is_empty() {
                problems.push("currency.rates has an empty currency code".to_string());
            }
            if !rate.is_finite() || *rate <= 0.0 {
                problems.push(format!(
                    "currency.rates['{code}'] must be a positive, finite rate (got {rate})"
                ));
            }
        }
        problems
    }
}

/// A [`CurrencyConverter`] over an operator-supplied rate table. Works
/// offline, which is the point.
#[derive(Debug, Clone, Default)]
pub struct StaticRates {
    config: CurrencyConfig,
}

impl StaticRates {
    pub fn new(config: CurrencyConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &CurrencyConfig {
        &self.config
    }
}

impl CurrencyConverter for StaticRates {
    fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64> {
        let (from, to) = (normalize_code(from), normalize_code(to));
        if from == to {
            return Some(amount);
        }
        // both legs go through the base, so a table of N codes converts any of
        // the N*N pairs without listing them
        let from_rate = self.config.rate(&from)?;
        let to_rate = self.config.rate(&to)?;
        if to_rate == 0.0 {
            return None;
        }
        Some(amount * from_rate / to_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rates() -> StaticRates {
        StaticRates::new(CurrencyConfig {
            base: "USD".to_string(),
            rates: HashMap::from([
                ("EUR".to_string(), 1.10),
                ("GBP".to_string(), 1.25),
                // an open code set: no enum lists this, only the table does
                ("BTC".to_string(), 60_000.0),
            ]),
        })
    }

    #[test]
    fn normalizes_currency_codes() {
        assert_eq!(normalize_code("usd"), "USD");
        assert_eq!(normalize_code("USD"), "USD");
        assert_eq!(normalize_code("Usd"), "USD");
        assert_eq!(normalize_code(" usd "), "USD");
        assert_eq!(normalize_code(""), "");
        assert_eq!(normalize_code("   "), "");
    }

    #[test]
    fn converts_into_and_out_of_the_base() {
        let fx = rates();
        assert_eq!(fx.convert(10.0, "EUR", "USD"), Some(11.0));
        assert_eq!(fx.convert(11.0, "USD", "EUR"), Some(10.0));
        assert_eq!(fx.convert(1.0, "USD", "USD"), Some(1.0));
    }

    #[test]
    fn converts_between_two_non_base_currencies() {
        let converted = rates().convert(100.0, "GBP", "EUR").unwrap();
        assert!((converted - 125.0 / 1.10).abs() < 1e-9, "{converted}");
    }

    #[test]
    fn a_new_code_needs_only_a_table_entry() {
        // the acceptance criterion from #650: no code change, no enum edit
        let fx = rates();
        assert_eq!(fx.convert(2.0, "BTC", "USD"), Some(120_000.0));
    }

    #[test]
    fn codes_are_case_and_whitespace_insensitive() {
        assert_eq!(rates().convert(10.0, " eur ", "usd"), Some(11.0));
    }

    #[test]
    fn an_unknown_pair_is_none_rather_than_a_guess() {
        // the whole point: the caller must be able to fail closed
        assert_eq!(rates().convert(10.0, "XYZ", "USD"), None);
        assert_eq!(rates().convert(10.0, "USD", "XYZ"), None);
    }

    #[test]
    fn problems_reject_an_unusable_table() {
        let bad = CurrencyConfig {
            base: "USD".to_string(),
            rates: HashMap::from([("EUR".to_string(), 0.0)]),
        };
        assert_eq!(bad.problems().len(), 1, "{:?}", bad.problems());

        let negative = CurrencyConfig {
            base: "USD".to_string(),
            rates: HashMap::from([("EUR".to_string(), -1.0)]),
        };
        assert_eq!(negative.problems().len(), 1);

        assert!(CurrencyConfig::default().problems().is_empty());
    }
}

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

    /// Every currency this deployment can price in: the base plus each code in
    /// the rate table, normalized and deduplicated, base first and the rest in
    /// alphabetical order.
    ///
    /// This is exactly the set [`Self::rate`] answers for, which is what makes
    /// it safe to offer as a chooser: the dashboard used to hardcode seven
    /// ISO-4217 codes, so a configured `RUB` was unselectable while an offered
    /// `JPY` with no rate was rejected on save (#965). Deriving both the
    /// chooser and the validator from one function means neither can drift.
    pub fn codes(&self) -> Vec<String> {
        let base = self.base_code();
        let mut rest: Vec<String> = self
            .rates
            .keys()
            .map(|code| normalize_code(code))
            .filter(|code| !code.is_empty() && *code != base)
            .collect();
        rest.sort();
        rest.dedup();
        // the base leads because it is the currency spend settles in, and an
        // operator picking "the default" should land on it without hunting
        std::iter::once(base).chain(rest).collect()
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
    fn codes_lead_with_the_base_then_sort() {
        assert_eq!(rates().config.codes(), ["USD", "BTC", "EUR", "GBP"]);
    }

    #[test]
    fn codes_are_exactly_the_set_that_has_a_rate() {
        // the invariant the dashboard chooser rests on: everything offered is
        // priceable, and everything priceable is offered
        let fx = rates();
        for code in fx.config.codes() {
            assert!(
                fx.config.rate(&code).is_some(),
                "offered but unpriceable: {code}"
            );
        }
        assert!(fx.config.rate("XYZ").is_none());
        assert!(!fx.config.codes().contains(&"XYZ".to_string()));
    }

    #[test]
    fn codes_are_not_a_fixed_set() {
        // #965: adding a currency must cost a rate-table entry and nothing else
        let mut config = CurrencyConfig::default();
        assert_eq!(config.codes(), ["USD"]);
        config.rates.insert("RUB".to_string(), 0.011);
        assert_eq!(config.codes(), ["USD", "RUB"]);
    }

    #[test]
    fn codes_normalize_and_never_repeat_the_base() {
        let config = CurrencyConfig {
            base: " usd ".to_string(),
            rates: HashMap::from([
                // the base needs no row, but listing it must not double it up
                ("usd".to_string(), 1.0),
                (" rub ".to_string(), 0.011),
                // an empty code is a config defect, not a currency to offer
                ("".to_string(), 2.0),
            ]),
        };
        assert_eq!(config.codes(), ["USD", "RUB"]);
    }

    #[test]
    fn a_non_base_currency_is_offered_only_once_it_has_a_rate() {
        let mut config = CurrencyConfig::default();
        assert!(!config.codes().contains(&"EUR".to_string()));
        config.rates.insert("EUR".to_string(), 1.09);
        assert!(config.codes().contains(&"EUR".to_string()));
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

//! The per-run token receipt (`vajra.token_receipt/1`).
//!
//! What a turn spent, by category, as the provider's own `usage` fields report it. Nothing is estimated and no price is
//! guessed: an absent field reads 0, and a negative or non-integer one is dropped, so a malformed event can never make the
//! receipt claim more or less than the provider said. The receipt is written into the turn's ledger line, where the
//! signed run record can pick it up.
//!
//! `side` counts tokens Vajra itself spent on calls the user did not ask for (titles, branch names). Vajra makes none, so
//! it is always 0; it is a field so a future regression shows up in a record instead of in a bill.

use serde_json::{json, Value};

pub const SCHEMA: &str = "vajra.token_receipt/1";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenReceipt {
    /// Input tokens that were not served from the cache.
    pub fresh: u64,
    /// Tokens written to the cache.
    pub cache_write: u64,
    /// Tokens read back from the cache.
    pub cache_read: u64,
    pub output: u64,
    /// Always 0 from Vajra (see the module note).
    pub side: u64,
}

impl TokenReceipt {
    /// Read a `result` event's `usage` object. `None` when the event carries no usage at all.
    pub fn from_usage(usage: &Value) -> Option<TokenReceipt> {
        let obj = usage.as_object()?;
        let n = |k: &str| obj.get(k).and_then(Value::as_u64).unwrap_or(0);
        Some(TokenReceipt {
            fresh: n("input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
            cache_read: n("cache_read_input_tokens"),
            output: n("output_tokens"),
            side: 0,
        })
    }

    /// Every token the turn handled, in any category.
    pub fn total(&self) -> u64 {
        [self.fresh, self.cache_write, self.cache_read, self.output, self.side].iter().fold(0u64, |a, b| a.saturating_add(*b))
    }

    /// Add another turn's receipt (a goal's receipt is the sum of its turns).
    pub fn add(&mut self, o: &TokenReceipt) {
        self.fresh = self.fresh.saturating_add(o.fresh);
        self.cache_write = self.cache_write.saturating_add(o.cache_write);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
        self.output = self.output.saturating_add(o.output);
        self.side = self.side.saturating_add(o.side);
    }

    pub fn to_json(&self) -> Value {
        json!({"schema": SCHEMA, "fresh": self.fresh, "cache_write": self.cache_write, "cache_read": self.cache_read,
               "output": self.output, "side": self.side, "total": self.total()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_providers_usage_fields_by_category() {
        let u = json!({"input_tokens": 12, "cache_creation_input_tokens": 300, "cache_read_input_tokens": 4000, "output_tokens": 55});
        let r = TokenReceipt::from_usage(&u).unwrap();
        assert_eq!(r, TokenReceipt { fresh: 12, cache_write: 300, cache_read: 4000, output: 55, side: 0 });
        assert_eq!(r.total(), 4367);
    }

    #[test]
    fn a_missing_or_malformed_field_reads_zero_and_no_usage_is_none() {
        let r = TokenReceipt::from_usage(&json!({"input_tokens": -5, "output_tokens": "9", "cache_read_input_tokens": 1.5})).unwrap();
        assert_eq!(r, TokenReceipt::default());
        assert!(TokenReceipt::from_usage(&json!(null)).is_none());
        assert!(TokenReceipt::from_usage(&json!([1, 2])).is_none());
    }

    #[test]
    fn the_json_carries_the_schema_and_a_total_equal_to_the_sum() {
        let r = TokenReceipt { fresh: 1, cache_write: 2, cache_read: 3, output: 4, side: 0 };
        let j = r.to_json();
        assert_eq!(j["schema"], SCHEMA);
        assert_eq!(j["total"], 10);
        assert_eq!(j["side"], 0);
    }

    #[test]
    fn adding_turns_sums_each_category_and_never_overflows() {
        let mut a = TokenReceipt { fresh: u64::MAX, output: 1, ..Default::default() };
        a.add(&TokenReceipt { fresh: 5, output: 2, ..Default::default() });
        assert_eq!(a.fresh, u64::MAX);
        assert_eq!(a.output, 3);
        assert_eq!(a.total(), u64::MAX);
    }
}

//! List prices of the Claude models on the Claude API, to estimate what the
//! saved token usage would cost. From the pricing page
//! (`platform.claude.com/docs/en/about-claude/pricing`), checked on 2026-11-13.
//! Other agents' models have no price here: their usage is not estimated.
//!
//! Not counted, because the saved figures don't say it per request: fast
//! mode, US-only inference (1.1×), Claude Haiku 5.5 prompts over 100K tokens,
//! web searches; nor the discounts of a contract.

use crate::usage::Tokens;

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub output: f64,
}

/// Base input, cache read and output; a cache write costs 1.25× the input for
/// 5 minutes, 2× for an hour.
const PRICES: &[(&str, f64, f64, f64)] = &[
    ("claude-fable-5-2", 8.0, 0.20, 40.0),
    ("claude-mythos-5-2", 8.0, 0.20, 40.0),
    ("claude-fable-5-1", 10.0, 0.25, 50.0),
    ("claude-mythos-5-1", 10.0, 0.25, 50.0),
    ("claude-fable-5", 10.0, 1.0, 50.0),
    ("claude-mythos-5", 10.0, 1.0, 50.0),
    ("claude-opus-5-5", 4.0, 0.20, 20.0),
    ("claude-opus-5", 5.0, 0.50, 25.0),
    ("claude-opus-4-8", 5.0, 0.50, 25.0),
    ("claude-opus-4-7", 5.0, 0.50, 25.0),
    ("claude-opus-4-6", 5.0, 0.50, 25.0),
    ("claude-opus-4-5", 5.0, 0.50, 25.0),
    ("claude-opus-4-1", 15.0, 1.50, 75.0),
    ("claude-opus-4", 15.0, 1.50, 75.0),
    ("claude-sonnet-5-5", 2.0, 0.10, 10.0),
    ("claude-sonnet-5", 2.0, 0.20, 10.0),
    ("claude-sonnet-4-6", 3.0, 0.30, 15.0),
    ("claude-sonnet-4-5", 3.0, 0.30, 15.0),
    ("claude-sonnet-4", 3.0, 0.30, 15.0),
    ("claude-haiku-5-5", 0.10, 0.01, 0.50),
    ("claude-haiku-4-5", 1.0, 0.10, 5.0),
];

/// `claude-haiku-4-5-20251001`, `claude-opus-4-5@20251101`,
/// `us.anthropic.claude-opus-4-5-20251101-v1:0`, `claude-sonnet-5-5[1m]` →
/// the model id without platform prefix, date and variant.
fn base_id(model: &str) -> Option<&str> {
    let s = &model[model.find("claude-")?..];
    let mut s = s.split(['@', '[', ':']).next().unwrap_or(s);
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    if let Some((head, _)) = s.rsplit_once("-v").filter(|(_, v)| digits(v)) {
        s = head;
    }
    if let Some((head, _)) = s.rsplit_once('-').filter(|(_, d)| d.len() == 8 && digits(d)) {
        s = head;
    }
    Some(s)
}

/// `None` for the models without a price (other agents, unknown Claude models).
pub fn price(model: &str) -> Option<Price> {
    let id = base_id(model)?;
    PRICES.iter().find(|p| p.0 == id).map(|&(_, input, cache_read, output)| Price {
        input,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read,
        output,
    })
}

/// Estimated cost in USD; `None` for the models without a price.
pub fn cost(model: &str, t: &Tokens) -> Option<f64> {
    let p = price(model)?;
    let write_5m = t.cache_write.saturating_sub(t.cache_write_1h);
    let usd = t.input as f64 * p.input
        + write_5m as f64 * p.cache_write_5m
        + t.cache_write_1h as f64 * p.cache_write_1h
        + t.cache_read as f64 * p.cache_read
        + t.output as f64 * p.output;
    Some(usd / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_from_every_platform() {
        assert_eq!(base_id("claude-opus-5-5"), Some("claude-opus-5-5"));
        assert_eq!(base_id("claude-haiku-4-5-20251001"), Some("claude-haiku-4-5"));
        assert_eq!(base_id("claude-opus-4-5@20251101"), Some("claude-opus-4-5"));
        assert_eq!(base_id("us.anthropic.claude-opus-4-5-20251101-v1:0"), Some("claude-opus-4-5"));
        assert_eq!(base_id("claude-sonnet-5-5[1m]"), Some("claude-sonnet-5-5"));
        assert_eq!(base_id("gpt-6-astra"), None);
    }

    #[test]
    fn prices_follow_the_model_not_its_prefix() {
        assert_eq!(price("claude-opus-4-20250514").map(|p| p.input), Some(15.0));
        assert_eq!(price("claude-opus-4-5").map(|p| p.input), Some(5.0));
        assert_eq!(price("claude-sonnet-5").map(|p| p.cache_read), Some(0.20));
        assert_eq!(price("claude-fable-5-2[1m]").map(|p| p.output), Some(40.0));
        assert_eq!(price("claude-opus-9-9"), None);
        assert_eq!(price("unknown"), None);
    }

    #[test]
    fn cost_splits_the_cache_writes_by_duration() {
        let t = Tokens {
            input: 1_000_000,
            cache_write: 3_000_000,
            cache_write_1h: 1_000_000,
            cache_read: 10_000_000,
            output: 1_000_000,
            reasoning: 400_000,
        };
        // Opus 5.5: 4 + 2 × 5 + 8 + 10 × 0.20 + 20.
        assert!((cost("claude-opus-5-5", &t).unwrap() - 44.0).abs() < 1e-9);
        assert_eq!(cost("gpt-6-astra", &t), None);
    }
}

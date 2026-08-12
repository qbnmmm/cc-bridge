use std::collections::BTreeMap;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::model::usage::{UsageCosts, UsageTokens};

pub const BASE_PRICING_VERSION: &str =
    "ccusage-bd72512910e2953650a4ca2e807953342b339804-2026-08-12";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PricingRule {
    model_key: String,
    input: Option<i64>,
    output: Option<i64>,
    cache_creation_5m: Option<i64>,
    cache_creation_1h: Option<i64>,
    cache_read: Option<i64>,
    long_context: Option<LongContextPricing>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LongContextPricing {
    threshold: i64,
    input: Option<i64>,
    output: Option<i64>,
    cache_creation_5m: Option<i64>,
    cache_creation_1h: Option<i64>,
    cache_read: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PricingEngine {
    rules: BTreeMap<String, PricingRule>,
    overrides: BTreeMap<String, PricingRule>,
    version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricedUsage {
    pub costs: UsageCosts,
    pub pricing_version: String,
    pub pricing_model_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OverrideRule {
    input_usd_per_million: Option<String>,
    output_usd_per_million: Option<String>,
    cache_creation_5m_usd_per_million: Option<String>,
    cache_creation_1h_usd_per_million: Option<String>,
    cache_read_usd_per_million: Option<String>,
    long_context: Option<OverrideLongContext>,
}

#[derive(Debug, Deserialize)]
struct OverrideLongContext {
    threshold: i64,
    input_usd_per_million: Option<String>,
    output_usd_per_million: Option<String>,
    cache_creation_5m_usd_per_million: Option<String>,
    cache_creation_1h_usd_per_million: Option<String>,
    cache_read_usd_per_million: Option<String>,
}

impl PricingEngine {
    pub fn from_override_json(override_json: Option<&str>) -> Result<Self, String> {
        let rules = builtin_rules();
        let mut overrides: BTreeMap<String, PricingRule> = BTreeMap::new();
        let mut version = BASE_PRICING_VERSION.to_string();
        if let Some(raw) = override_json.map(str::trim).filter(|raw| !raw.is_empty()) {
            let input_overrides: BTreeMap<String, OverrideRule> = serde_json::from_str(raw)
                .map_err(|err| format!("invalid USAGE_PRICING_OVERRIDES_JSON: {err}"))?;
            for (model, rule) in input_overrides {
                let model = model.trim().to_ascii_lowercase();
                if model.is_empty() {
                    return Err("pricing override model must not be empty".into());
                }
                overrides.insert(model.clone(), parse_override_rule(model, rule)?);
            }
            let canonical = serde_json::to_vec(
                &serde_json::from_str::<serde_json::Value>(raw)
                    .map_err(|err| format!("invalid pricing override JSON: {err}"))?,
            )
            .map_err(|err| format!("canonicalize pricing overrides: {err}"))?;
            let digest = Sha256::digest(canonical);
            version = format!(
                "{}+override-{}",
                BASE_PRICING_VERSION,
                hex::encode(&digest[..8])
            );
        }
        Ok(Self {
            rules,
            overrides,
            version,
        })
    }

    pub fn price(&self, model: &str, tokens: &UsageTokens) -> PricedUsage {
        let matched = self.find_rule(model);
        let rates = matched.map(|rule| {
            if rule
                .long_context
                .as_ref()
                .is_some_and(|tier| tokens.input > tier.threshold)
            {
                let tier = rule.long_context.as_ref().expect("checked above");
                (
                    tier.input,
                    tier.output,
                    tier.cache_creation_5m,
                    tier.cache_creation_1h,
                    tier.cache_read,
                )
            } else {
                (
                    rule.input,
                    rule.output,
                    rule.cache_creation_5m,
                    rule.cache_creation_1h,
                    rule.cache_read,
                )
            }
        });
        let (input_rate, output_rate, cache_5m_rate, cache_1h_rate, cache_read_rate) =
            rates.unwrap_or((None, None, None, None, None));

        let input = component_cost(tokens.input, input_rate);
        let output = component_cost(tokens.output, output_rate);
        let cache_5m = component_cost(tokens.cache_creation_5m, cache_5m_rate);
        let cache_1h = component_cost(tokens.cache_creation_1h, cache_1h_rate);
        let cache_read = component_cost(tokens.cache_read, cache_read_rate);

        let known_nano_usd = [input, output, cache_5m, cache_1h, cache_read]
            .into_iter()
            .flatten()
            .fold(0_i64, i64::saturating_add);
        let unpriced_tokens = [
            (tokens.input, input),
            (tokens.output, output),
            (tokens.cache_creation_5m, cache_5m),
            (tokens.cache_creation_1h, cache_1h),
            (tokens.cache_read, cache_read),
        ]
        .into_iter()
        .filter_map(|(count, cost)| (count > 0 && cost.is_none()).then_some(count))
        .fold(0_i64, i64::saturating_add);

        PricedUsage {
            costs: UsageCosts {
                input_nano_usd: input,
                output_nano_usd: output,
                cache_creation_5m_nano_usd: cache_5m,
                cache_creation_1h_nano_usd: cache_1h,
                cache_read_nano_usd: cache_read,
                known_nano_usd,
                complete: unpriced_tokens == 0,
                unpriced_tokens,
            },
            pricing_version: self.version.clone(),
            pricing_model_key: matched.map(|rule| rule.model_key.clone()),
        }
    }

    fn find_rule(&self, model: &str) -> Option<&PricingRule> {
        let exact = model.trim().to_ascii_lowercase();
        let normalized = exact.replace('.', "-");
        self.overrides
            .get(&exact)
            .or_else(|| self.rules.get(&normalized))
            .or_else(|| {
                self.rules
                    .iter()
                    .filter(|(key, _)| model_key_matches(&normalized, key))
                    .max_by_key(|(key, _)| key.len())
                    .map(|(_, rule)| rule)
            })
    }
}

fn component_cost(tokens: i64, nano_usd_per_million: Option<i64>) -> Option<i64> {
    if tokens <= 0 {
        return Some(0);
    }
    let rate = nano_usd_per_million?;
    let numerator = i128::from(tokens).checked_mul(i128::from(rate))?;
    let rounded = numerator.checked_add(500_000)?.checked_div(1_000_000)?;
    i64::try_from(rounded).ok()
}

fn parse_override_rule(model_key: String, rule: OverrideRule) -> Result<PricingRule, String> {
    Ok(PricingRule {
        model_key,
        input: parse_optional_rate(rule.input_usd_per_million.as_deref())?,
        output: parse_optional_rate(rule.output_usd_per_million.as_deref())?,
        cache_creation_5m: parse_optional_rate(rule.cache_creation_5m_usd_per_million.as_deref())?,
        cache_creation_1h: parse_optional_rate(rule.cache_creation_1h_usd_per_million.as_deref())?,
        cache_read: parse_optional_rate(rule.cache_read_usd_per_million.as_deref())?,
        long_context: rule
            .long_context
            .map(|tier| -> Result<LongContextPricing, String> {
                if tier.threshold <= 0 {
                    return Err("long_context.threshold must be positive".to_string());
                }
                Ok(LongContextPricing {
                    threshold: tier.threshold,
                    input: parse_optional_rate(tier.input_usd_per_million.as_deref())?,
                    output: parse_optional_rate(tier.output_usd_per_million.as_deref())?,
                    cache_creation_5m: parse_optional_rate(
                        tier.cache_creation_5m_usd_per_million.as_deref(),
                    )?,
                    cache_creation_1h: parse_optional_rate(
                        tier.cache_creation_1h_usd_per_million.as_deref(),
                    )?,
                    cache_read: parse_optional_rate(tier.cache_read_usd_per_million.as_deref())?,
                })
            })
            .transpose()?,
    })
}

fn parse_optional_rate(value: Option<&str>) -> Result<Option<i64>, String> {
    value.map(parse_usd_per_million).transpose()
}

fn parse_usd_per_million(value: &str) -> Result<i64, String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.starts_with('+') {
        return Err(format!("invalid non-negative USD rate: {value}"));
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 9
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err(format!("invalid USD rate with at most 9 decimals: {value}"));
    }
    let whole: i128 = whole
        .parse()
        .map_err(|_| format!("USD rate out of range: {value}"))?;
    let fraction: i128 = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i128>()
            .map_err(|_| format!("invalid USD rate: {value}"))?
            * 10_i128.pow((9 - fraction.len()) as u32)
    };
    let nano_usd = whole
        .checked_mul(1_000_000_000)
        .and_then(|whole| whole.checked_add(fraction))
        .ok_or_else(|| format!("USD rate out of range: {value}"))?;
    i64::try_from(nano_usd).map_err(|_| format!("USD rate out of range: {value}"))
}

fn model_key_matches(model: &str, key: &str) -> bool {
    model == key
        || model.strip_prefix(key).is_some_and(|suffix| {
            suffix.starts_with('-')
                && suffix[1..].len() == 8
                && suffix[1..].chars().all(|c| c.is_ascii_digit())
        })
}

fn rule(
    model_key: &str,
    input: &str,
    output: &str,
    cache_5m: &str,
    cache_1h: &str,
    cache_read: &str,
) -> PricingRule {
    PricingRule {
        model_key: model_key.into(),
        input: Some(parse_usd_per_million(input).expect("builtin input pricing")),
        output: Some(parse_usd_per_million(output).expect("builtin output pricing")),
        cache_creation_5m: Some(parse_usd_per_million(cache_5m).expect("builtin cache 5m pricing")),
        cache_creation_1h: Some(parse_usd_per_million(cache_1h).expect("builtin cache 1h pricing")),
        cache_read: Some(parse_usd_per_million(cache_read).expect("builtin cache read pricing")),
        long_context: None,
    }
}

fn builtin_rules() -> BTreeMap<String, PricingRule> {
    let mut rules: BTreeMap<String, PricingRule> = [
        rule("claude-opus-4-6", "5", "25", "6.25", "10", "0.5"),
        rule("claude-opus-4-5", "5", "25", "6.25", "10", "0.5"),
        rule("claude-opus-4-1", "15", "75", "18.75", "30", "1.5"),
        rule("claude-opus-4", "15", "75", "18.75", "30", "1.5"),
        rule("claude-sonnet-4-6", "3", "15", "3.75", "6", "0.3"),
        rule("claude-sonnet-4-5", "3", "15", "3.75", "6", "0.3"),
        rule("claude-sonnet-4", "3", "15", "3.75", "6", "0.3"),
        rule("claude-haiku-4-5", "1", "5", "1.25", "2", "0.1"),
        rule("claude-3-7-sonnet", "3", "15", "3.75", "6", "0.3"),
        rule("claude-3-5-sonnet", "3", "15", "3.75", "6", "0.3"),
        rule("claude-3-5-haiku", "0.8", "4", "1", "1.6", "0.08"),
        rule("claude-3-opus", "15", "75", "18.75", "30", "1.5"),
        rule("claude-3-haiku", "0.25", "1.25", "0.3", "0.5", "0.03"),
    ]
    .into_iter()
    .map(|rule| (rule.model_key.clone(), rule))
    .collect();
    for key in ["claude-sonnet-4", "claude-sonnet-4-5", "claude-sonnet-4-6"] {
        if let Some(rule) = rules.get_mut(key) {
            rule.long_context = Some(LongContextPricing {
                threshold: 200_000,
                input: Some(parse_usd_per_million("6").expect("builtin long input")),
                output: Some(parse_usd_per_million("22.5").expect("builtin long output")),
                cache_creation_5m: Some(
                    parse_usd_per_million("7.5").expect("builtin long cache 5m"),
                ),
                cache_creation_1h: Some(
                    parse_usd_per_million("12").expect("builtin long cache 1h"),
                ),
                cache_read: Some(parse_usd_per_million("0.6").expect("builtin long cache read")),
            });
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_all_components_in_nano_usd() {
        let engine = PricingEngine::from_override_json(None).unwrap();
        let priced = engine.price(
            "claude-sonnet-4-6",
            &UsageTokens {
                input: 200_000,
                output: 1_000_000,
                cache_creation_5m: 1_000_000,
                cache_creation_1h: 1_000_000,
                cache_read: 1_000_000,
            },
        );
        assert_eq!(priced.costs.input_nano_usd, Some(600_000_000));
        assert_eq!(priced.costs.output_nano_usd, Some(15_000_000_000));
        assert_eq!(priced.costs.cache_creation_5m_nano_usd, Some(3_750_000_000));
        assert_eq!(priced.costs.cache_creation_1h_nano_usd, Some(6_000_000_000));
        assert_eq!(priced.costs.cache_read_nano_usd, Some(300_000_000));
        assert!(priced.costs.complete);
    }

    #[test]
    fn unknown_model_keeps_tokens_unpriced() {
        let engine = PricingEngine::from_override_json(None).unwrap();
        let priced = engine.price(
            "unknown-model",
            &UsageTokens {
                input: 7,
                ..UsageTokens::default()
            },
        );
        assert_eq!(priced.costs.input_nano_usd, None);
        assert_eq!(priced.costs.unpriced_tokens, 7);
        assert!(!priced.costs.complete);
    }

    #[test]
    fn sonnet_uses_long_context_rates_above_200k() {
        let engine = PricingEngine::from_override_json(None).unwrap();
        let priced = engine.price(
            "claude-sonnet-4-6",
            &UsageTokens {
                input: 200_001,
                output: 1_000_000,
                ..UsageTokens::default()
            },
        );
        assert_eq!(priced.costs.output_nano_usd, Some(22_500_000_000));
    }

    #[test]
    fn date_suffix_matches_but_numeric_version_does_not() {
        let engine = PricingEngine::from_override_json(None).unwrap();
        let tokens = UsageTokens {
            input: 1,
            ..UsageTokens::default()
        };
        assert!(
            engine
                .price("claude-sonnet-4-6-20260416", &tokens)
                .pricing_model_key
                .is_some()
        );
        assert!(
            engine
                .price("claude-sonnet-4-60", &tokens)
                .pricing_model_key
                .is_none()
        );
    }

    #[test]
    fn exact_override_supports_partial_pricing() {
        let engine = PricingEngine::from_override_json(Some(
            r#"{"custom":{"input_usd_per_million":"2.5"}}"#,
        ))
        .unwrap();
        let priced = engine.price(
            "custom",
            &UsageTokens {
                input: 2,
                output: 1,
                ..UsageTokens::default()
            },
        );
        assert_eq!(priced.costs.input_nano_usd, Some(5_000));
        assert_eq!(priced.costs.output_nano_usd, None);
        assert_eq!(priced.costs.unpriced_tokens, 1);
        assert!(priced.pricing_version.contains("+override-"));
        assert!(
            engine
                .price(
                    "custom-20260812",
                    &UsageTokens {
                        input: 1,
                        ..UsageTokens::default()
                    }
                )
                .pricing_model_key
                .is_none()
        );
    }

    #[test]
    fn exact_override_preserves_dot_in_model_name() {
        let engine = PricingEngine::from_override_json(Some(
            r#"{"claude-opus-4.7":{"input_usd_per_million":"42"}}"#,
        ))
        .unwrap();
        let priced = engine.price(
            "claude-opus-4.7",
            &UsageTokens {
                input: 1,
                ..UsageTokens::default()
            },
        );
        assert_eq!(priced.costs.input_nano_usd, Some(42_000));
        assert_eq!(priced.pricing_model_key.as_deref(), Some("claude-opus-4.7"));
    }

    #[test]
    fn invalid_override_is_rejected() {
        let err =
            PricingEngine::from_override_json(Some(r#"{"custom":{"input_usd_per_million":"-1"}}"#))
                .unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn overflowing_override_is_rejected_without_panicking() {
        let err = PricingEngine::from_override_json(Some(&format!(
            r#"{{"custom":{{"input_usd_per_million":"{}"}}}}"#,
            "9".repeat(80)
        )))
        .unwrap_err();
        assert!(err.contains("out of range"));
    }
}

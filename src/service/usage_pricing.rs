use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::model::usage::{UsageCosts, UsageTokens};

pub const BASE_PRICING_VERSION: &str = "builtin-anthropic-2026-08-13";
const LITELLM_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const MODELS_DEV_PRICING_URL: &str = "https://models.dev/api.json";
const PRICING_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const PRICING_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PricingRule {
    model_key: String,
    input: Option<i64>,
    output: Option<i64>,
    cache_creation_5m: Option<i64>,
    cache_creation_1h: Option<i64>,
    cache_read: Option<i64>,
    long_context: Option<LongContextPricing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    override_version_suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingSnapshot {
    version: String,
    rules: BTreeMap<String, PricingRule>,
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
        let mut override_version_suffix = String::new();
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
            override_version_suffix = format!("+override-{}", hex::encode(&digest[..8]));
        }
        Ok(Self {
            rules,
            overrides,
            version: format!("{BASE_PRICING_VERSION}{override_version_suffix}"),
            override_version_suffix,
        })
    }

    pub fn with_snapshot(&self, snapshot: &PricingSnapshot) -> Result<Self, String> {
        snapshot.validate()?;
        let mut rules = builtin_rules();
        for (key, rule) in &snapshot.rules {
            let mut merged = rule.clone();
            if merged.long_context.is_none() {
                merged.long_context = rules
                    .get(key)
                    .and_then(|existing| existing.long_context.clone());
            }
            rules.insert(key.clone(), merged);
        }
        Ok(Self {
            rules,
            overrides: self.overrides.clone(),
            version: format!("{}{}", snapshot.version, self.override_version_suffix),
            override_version_suffix: self.override_version_suffix.clone(),
        })
    }

    pub fn version(&self) -> &str {
        &self.version
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

impl PricingSnapshot {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_str(json)
            .map_err(|error| format!("invalid cached pricing snapshot: {error}"))?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| format!("serialize pricing snapshot: {error}"))
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    fn validate(&self) -> Result<(), String> {
        if self.version.trim().is_empty() || self.rules.is_empty() {
            return Err("pricing snapshot must contain a version and at least one rule".into());
        }
        for (key, rule) in &self.rules {
            if key.trim().is_empty()
                || rule.model_key != *key
                || [
                    rule.input,
                    rule.output,
                    rule.cache_creation_5m,
                    rule.cache_creation_1h,
                    rule.cache_read,
                ]
                .into_iter()
                .flatten()
                .any(|rate| rate < 0)
            {
                return Err(format!("invalid pricing rule for model {key}"));
            }
            if rule.long_context.as_ref().is_some_and(|tier| {
                tier.threshold <= 0
                    || [
                        tier.input,
                        tier.output,
                        tier.cache_creation_5m,
                        tier.cache_creation_1h,
                        tier.cache_read,
                    ]
                    .into_iter()
                    .flatten()
                    .any(|rate| rate < 0)
            }) {
                return Err(format!("invalid long-context pricing rule for model {key}"));
            }
        }
        Ok(())
    }
}

pub async fn fetch_remote_pricing_snapshot() -> Result<PricingSnapshot, String> {
    let client = pricing_http_client()?;
    let (litellm, models_dev) = tokio::join!(
        fetch_pricing_json_with_timeout(&client, LITELLM_PRICING_URL),
        fetch_pricing_json_with_timeout(&client, MODELS_DEV_PRICING_URL),
    );

    let mut rules = BTreeMap::new();
    let mut sources = Vec::new();
    match litellm.and_then(|json| parse_litellm_snapshot(&json)) {
        Ok(parsed) => {
            sources.push("litellm");
            rules.extend(parsed);
        }
        Err(error) => tracing::warn!("LiteLLM pricing refresh unavailable: {}", error),
    }
    match models_dev.and_then(|json| parse_models_dev_snapshot(&json)) {
        Ok(parsed) => {
            sources.push("modelsdev");
            for (key, rule) in parsed {
                rules.entry(key).or_insert(rule);
            }
        }
        Err(error) => tracing::warn!("models.dev pricing refresh unavailable: {}", error),
    }
    if rules.is_empty() {
        return Err("all remote pricing sources failed or returned no Anthropic rules".into());
    }

    let canonical = serde_json::to_vec(&rules)
        .map_err(|error| format!("canonicalize remote pricing: {error}"))?;
    let digest = Sha256::digest(canonical);
    let snapshot = PricingSnapshot {
        version: format!("auto-{}-{}", sources.join("-"), hex::encode(&digest[..8])),
        rules,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

async fn fetch_pricing_json_with_timeout(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, String> {
    tokio::time::timeout(PRICING_HTTP_TIMEOUT, fetch_pricing_json(client, url))
        .await
        .map_err(|_| "request exceeded 20 second total timeout".to_string())?
}

fn pricing_http_client() -> Result<reqwest::Client, String> {
    crate::tlsfp::make_request_client_with_timeouts(PRICING_HTTP_TIMEOUT, PRICING_HTTP_TIMEOUT)
        .map_err(|error| format!("build pricing HTTP client: {error}"))
}

async fn fetch_pricing_json(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("HTTP error: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > PRICING_RESPONSE_LIMIT as u64)
    {
        return Err("response exceeds 8 MiB limit".into());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("read response: {error}"))?;
        if body.len().saturating_add(chunk.len()) > PRICING_RESPONSE_LIMIT {
            return Err("response exceeds 8 MiB limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "response is not UTF-8 JSON".into())
}

fn parse_litellm_snapshot(json: &str) -> Result<BTreeMap<String, PricingRule>, String> {
    let root: BTreeMap<String, Value> = serde_json::from_str(json)
        .map_err(|error| format!("invalid LiteLLM pricing JSON: {error}"))?;
    let mut rules = BTreeMap::new();
    for (model, value) in root {
        let Some(entry) = value.as_object() else {
            continue;
        };
        if entry.get("litellm_provider").and_then(Value::as_str) != Some("anthropic")
            || !model.starts_with("claude-")
        {
            continue;
        }
        let Some(input) = json_rate(entry.get("input_cost_per_token"), 15)? else {
            continue;
        };
        let Some(output) = json_rate(entry.get("output_cost_per_token"), 15)? else {
            continue;
        };
        let cache_creation_5m = json_rate(entry.get("cache_creation_input_token_cost"), 15)?
            .or_else(|| multiply_rate(input, 5, 4));
        let cache_creation_1h =
            json_rate(entry.get("cache_creation_input_token_cost_above_1hr"), 15)?
                .or_else(|| multiply_rate(input, 2, 1));
        let cache_read = json_rate(entry.get("cache_read_input_token_cost"), 15)?
            .or_else(|| multiply_rate(input, 1, 10));

        let long_input = json_rate(entry.get("input_cost_per_token_above_200k_tokens"), 15)?;
        let long_output = json_rate(entry.get("output_cost_per_token_above_200k_tokens"), 15)?;
        let long_cache_creation = json_rate(
            entry.get("cache_creation_input_token_cost_above_200k_tokens"),
            15,
        )?;
        let long_cache_read = json_rate(
            entry.get("cache_read_input_token_cost_above_200k_tokens"),
            15,
        )?;
        let long_context = [
            long_input,
            long_output,
            long_cache_creation,
            long_cache_read,
        ]
        .into_iter()
        .any(|rate| rate.is_some())
        .then(|| LongContextPricing {
            threshold: 200_000,
            input: long_input.or(Some(input)),
            output: long_output.or(Some(output)),
            cache_creation_5m: long_cache_creation.or(cache_creation_5m),
            cache_creation_1h: long_input
                .and_then(|rate| multiply_rate(rate, 2, 1))
                .or(cache_creation_1h),
            cache_read: long_cache_read.or(cache_read),
        });
        let model_key = normalize_model_key(&model);
        rules.insert(
            model_key.clone(),
            PricingRule {
                model_key,
                input: Some(input),
                output: Some(output),
                cache_creation_5m,
                cache_creation_1h,
                cache_read,
                long_context,
            },
        );
    }
    if rules.is_empty() {
        return Err("LiteLLM returned no complete Anthropic pricing rules".into());
    }
    Ok(rules)
}

fn parse_models_dev_snapshot(json: &str) -> Result<BTreeMap<String, PricingRule>, String> {
    let root: Value = serde_json::from_str(json)
        .map_err(|error| format!("invalid models.dev pricing JSON: {error}"))?;
    let models = root
        .get("anthropic")
        .and_then(|provider| provider.get("models"))
        .and_then(Value::as_object)
        .ok_or_else(|| "models.dev response is missing anthropic.models".to_string())?;
    let mut rules = BTreeMap::new();
    for (key, model) in models {
        let Some(cost) = model.get("cost").and_then(Value::as_object) else {
            continue;
        };
        let Some(input) = json_rate(cost.get("input"), 9)? else {
            continue;
        };
        let Some(output) = json_rate(cost.get("output"), 9)? else {
            continue;
        };
        let cache_creation_5m =
            json_rate(cost.get("cache_write"), 9)?.or_else(|| multiply_rate(input, 5, 4));
        let cache_creation_1h = multiply_rate(input, 2, 1);
        let cache_read =
            json_rate(cost.get("cache_read"), 9)?.or_else(|| multiply_rate(input, 1, 10));
        let model_key = normalize_model_key(
            model
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(key.as_str()),
        );
        if !model_key.starts_with("claude-") {
            continue;
        }
        rules.insert(
            model_key.clone(),
            PricingRule {
                model_key,
                input: Some(input),
                output: Some(output),
                cache_creation_5m,
                cache_creation_1h,
                cache_read,
                long_context: None,
            },
        );
    }
    if rules.is_empty() {
        return Err("models.dev returned no complete Anthropic pricing rules".into());
    }
    Ok(rules)
}

fn normalize_model_key(model: &str) -> String {
    model.trim().to_ascii_lowercase().replace('.', "-")
}

fn json_rate(value: Option<&Value>, decimal_scale: i32) -> Result<Option<i64>, String> {
    value
        .map(|value| {
            let raw = match value {
                Value::Number(number) => number.to_string(),
                Value::String(value) => value.clone(),
                _ => return Err("pricing rate must be a JSON number or decimal string".into()),
            };
            parse_decimal_scaled(&raw, decimal_scale)
        })
        .transpose()
}

fn parse_decimal_scaled(value: &str, scale: i32) -> Result<i64, String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.starts_with('+') {
        return Err(format!("invalid non-negative pricing rate: {value}"));
    }
    let (mantissa, exponent) = match value.find(['e', 'E']) {
        Some(index) => {
            let exponent = value[index + 1..]
                .parse::<i32>()
                .map_err(|_| format!("invalid pricing exponent: {value}"))?;
            (&value[..index], exponent)
        }
        None => (value, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole.chars().all(|character| character.is_ascii_digit())
        || !fraction.chars().all(|character| character.is_ascii_digit())
    {
        return Err(format!("invalid pricing decimal: {value}"));
    }
    let digits = format!("{whole}{fraction}")
        .parse::<i128>()
        .map_err(|_| format!("pricing rate out of range: {value}"))?;
    let power = scale
        .checked_add(exponent)
        .and_then(|power| power.checked_sub(fraction.len() as i32))
        .ok_or_else(|| format!("pricing scale out of range: {value}"))?;
    let scaled = if power >= 0 {
        digits
            .checked_mul(
                10_i128
                    .checked_pow(power as u32)
                    .ok_or_else(|| format!("pricing rate out of range: {value}"))?,
            )
            .ok_or_else(|| format!("pricing rate out of range: {value}"))?
    } else {
        let magnitude = power
            .checked_abs()
            .ok_or_else(|| format!("pricing scale out of range: {value}"))?;
        let divisor = 10_i128
            .checked_pow(magnitude as u32)
            .ok_or_else(|| format!("pricing rate out of range: {value}"))?;
        digits
            .checked_add(divisor / 2)
            .ok_or_else(|| format!("pricing rate out of range: {value}"))?
            / divisor
    };
    i64::try_from(scaled).map_err(|_| format!("pricing rate out of range: {value}"))
}

fn multiply_rate(rate: i64, numerator: i64, denominator: i64) -> Option<i64> {
    i128::from(rate)
        .checked_mul(i128::from(numerator))?
        .checked_add(i128::from(denominator / 2))?
        .checked_div(i128::from(denominator))?
        .try_into()
        .ok()
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
        rule("claude-opus-5", "5", "25", "6.25", "10", "0.5"),
        rule("claude-opus-4-8", "5", "25", "6.25", "10", "0.5"),
        rule("claude-opus-4-7", "5", "25", "6.25", "10", "0.5"),
        rule("claude-opus-4-6", "5", "25", "6.25", "10", "0.5"),
        rule("claude-opus-4-5", "5", "25", "6.25", "10", "0.5"),
        rule("claude-opus-4-1", "15", "75", "18.75", "30", "1.5"),
        rule("claude-opus-4", "15", "75", "18.75", "30", "1.5"),
        rule("claude-sonnet-4-6", "3", "15", "3.75", "6", "0.3"),
        rule("claude-sonnet-4-5", "3", "15", "3.75", "6", "0.3"),
        rule("claude-sonnet-4", "3", "15", "3.75", "6", "0.3"),
        rule("claude-sonnet-5", "2", "10", "2.5", "4", "0.2"),
        rule("claude-fable-5", "10", "50", "12.5", "20", "1"),
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
    fn pricing_http_client_builds_with_project_rustls() {
        pricing_http_client().unwrap();
    }

    #[test]
    fn decimal_parser_rejects_minimum_exponent_without_panicking() {
        assert!(parse_decimal_scaled("0.000000000000000e-2147483648", 15).is_err());
    }

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
    fn builtin_snapshot_prices_opus_five() {
        let engine = PricingEngine::from_override_json(None).unwrap();
        let priced = engine.price(
            "claude-opus-5",
            &UsageTokens {
                input: 289,
                output: 2_027,
                cache_creation_5m: 6_099,
                cache_creation_1h: 0,
                cache_read: 4_272_647,
            },
        );
        assert_eq!(priced.costs.known_nano_usd, 2_226_562_250);
        assert_eq!(priced.pricing_model_key.as_deref(), Some("claude-opus-5"));
        assert!(priced.costs.complete);
    }

    #[test]
    fn parses_litellm_anthropic_rules_and_scientific_notation() {
        let rules = parse_litellm_snapshot(
            r#"{
              "claude-opus-5": {
                "litellm_provider": "anthropic",
                "input_cost_per_token": 0.000005,
                "output_cost_per_token": 0.000025,
                "cache_creation_input_token_cost": 0.00000625,
                "cache_creation_input_token_cost_above_1hr": 0.00001,
                "cache_read_input_token_cost": 5e-7
              },
              "claude-opus-5-third-party": {
                "litellm_provider": "bedrock",
                "input_cost_per_token": 0.5,
                "output_cost_per_token": 1.0
              }
            }"#,
        )
        .unwrap();
        assert_eq!(rules.len(), 1);
        let rule = &rules["claude-opus-5"];
        assert_eq!(rule.input, Some(5_000_000_000));
        assert_eq!(rule.output, Some(25_000_000_000));
        assert_eq!(rule.cache_creation_5m, Some(6_250_000_000));
        assert_eq!(rule.cache_creation_1h, Some(10_000_000_000));
        assert_eq!(rule.cache_read, Some(500_000_000));
    }

    #[test]
    fn models_dev_fills_cache_defaults_and_snapshot_round_trips() {
        let rules = parse_models_dev_snapshot(
            r#"{
              "anthropic": {"models": {
                "claude-new": {
                  "id": "claude-new",
                  "cost": {"input": 2, "output": 10}
                }
              }},
              "other": {"models": {
                "claude-new": {"cost": {"input": 99, "output": 99}}
              }}
            }"#,
        )
        .unwrap();
        let rule = &rules["claude-new"];
        assert_eq!(rule.cache_creation_5m, Some(2_500_000_000));
        assert_eq!(rule.cache_creation_1h, Some(4_000_000_000));
        assert_eq!(rule.cache_read, Some(200_000_000));

        let snapshot = PricingSnapshot {
            version: "test-remote-v1".into(),
            rules,
        };
        let restored = PricingSnapshot::from_json(&snapshot.to_json().unwrap()).unwrap();
        assert_eq!(restored, snapshot);
        let engine = PricingEngine::from_override_json(None)
            .unwrap()
            .with_snapshot(&restored)
            .unwrap();
        assert_eq!(engine.version(), "test-remote-v1");
        assert!(
            engine
                .price(
                    "claude-new",
                    &UsageTokens {
                        input: 1,
                        ..UsageTokens::default()
                    }
                )
                .costs
                .complete
        );
    }

    #[test]
    fn cached_snapshot_rejects_invalid_long_context_rule() {
        let mut rules = parse_models_dev_snapshot(
            r#"{"anthropic":{"models":{"claude-new":{"cost":{"input":2,"output":10}}}}}"#,
        )
        .unwrap();
        rules.get_mut("claude-new").unwrap().long_context = Some(LongContextPricing {
            threshold: 0,
            input: Some(-1),
            output: None,
            cache_creation_5m: None,
            cache_creation_1h: None,
            cache_read: None,
        });
        let snapshot = PricingSnapshot {
            version: "invalid-cache".into(),
            rules,
        };

        assert!(PricingSnapshot::from_json(&snapshot.to_json().unwrap()).is_err());
    }

    #[test]
    fn exact_override_remains_above_remote_snapshot() {
        let base = PricingEngine::from_override_json(Some(
            r#"{"claude-opus-5":{"input_usd_per_million":"42"}}"#,
        ))
        .unwrap();
        let snapshot = PricingSnapshot {
            version: "remote-v1".into(),
            rules: parse_models_dev_snapshot(
                r#"{"anthropic":{"models":{"claude-opus-5":{"cost":{"input":5,"output":25,"cache_write":6.25,"cache_read":0.5}}}}}"#,
            )
            .unwrap(),
        };
        let engine = base.with_snapshot(&snapshot).unwrap();
        let priced = engine.price(
            "claude-opus-5",
            &UsageTokens {
                input: 1,
                ..UsageTokens::default()
            },
        );
        assert_eq!(priced.costs.input_nano_usd, Some(42_000));
        assert!(engine.version().starts_with("remote-v1+override-"));
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

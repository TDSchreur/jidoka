//! Estimated cost of a run's token usage, computed from a price table at read time and never stored.
//!
//! A model without a price has an unknown cost, never zero. A total that mixes priced and unpriced models
//! is partial: the sum of what is known, marked as a lower bound.

use crate::{
    run::Stage,
    usage::{Tokens, Usage, UsageRow},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// US dollars per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Price {
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) cache_read: f64,
    pub(crate) cache_write: f64,
}

const fn price(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        cache_write,
    }
}

/// Built-in prices by exact model id, USD per million tokens (cache write is the 5-minute price).
///
/// Source: Anthropic API pricing as listed in the claude-api skill reference (model table, models guide and
/// prompt-caching guide), read on 2026-11-08. Cache reads are listed explicitly for Fable 5.1 ($0.25),
/// Opus 5.5 and Sonnet 5.5 ($0.20); Opus 4.8 uses the documented 0.1x input rule. Cache writes are the
/// documented 1.25x input price. Haiku 5.5 is left out (its reference lists tiered input/output prices and
/// no cache prices). No OpenAI, Codex or Copilot prices are listed: those models show as unknown.
pub(crate) const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5-1", price(10.0, 50.0, 0.25, 12.5)),
    ("claude-opus-5-5", price(4.0, 20.0, 0.20, 5.0)),
    ("claude-sonnet-5-5", price(2.0, 10.0, 0.20, 2.5)),
    ("claude-opus-4-8", price(5.0, 25.0, 0.50, 6.25)),
];

pub(crate) type Table<'a> = &'a [(&'a str, Price)];

fn lookup(table: Table, model: &str) -> Option<Price> {
    table.iter().find(|(id, _)| *id == model).map(|(_, p)| *p)
}

/// The cost of one set of tokens in dollars. `None` when neither input nor output was reported; counts
/// that were not reported otherwise add nothing.
pub(crate) fn tokens_cost(tokens: &Tokens, price: &Price) -> Option<f64> {
    if tokens.input.is_none() && tokens.output.is_none() {
        return None;
    }
    let part = |count: Option<u64>, per_million: f64| count.unwrap_or(0) as f64 * per_million;
    Some(
        (part(tokens.input, price.input)
            + part(tokens.output, price.output)
            + part(tokens.cache_read, price.cache_read)
            + part(tokens.cache_write, price.cache_write))
            / 1e6,
    )
}

/// A cost, or what is known of it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Cost {
    /// Dollars; `None` (JSON `null`) when nothing is priced. With `partial` it is a lower bound.
    pub(crate) usd: Option<f64>,
    /// Some contributing usage had no price or no countable tokens, so `usd` is too low.
    pub(crate) partial: bool,
    /// Models that had no price.
    pub(crate) unpriced_models: Vec<String>,
}

/// The cost of a row, `None` when its model has no price or it reported no input and output.
pub(crate) fn row_cost(row: &UsageRow, table: Table) -> Option<f64> {
    tokens_cost(&row.tokens, &lookup(table, &row.model)?)
}

/// The summed cost of rows.
pub(crate) fn total<'a>(rows: impl IntoIterator<Item = &'a UsageRow>, table: Table) -> Cost {
    let mut usd: Option<f64> = None;
    let mut unknown = false;
    let mut unpriced = BTreeSet::new();
    for row in rows {
        match row_cost(row, table) {
            Some(cost) => usd = Some(usd.unwrap_or(0.0) + cost),
            None => {
                unknown = true;
                if lookup(table, &row.model).is_none() {
                    unpriced.insert(row.model.clone());
                }
            }
        }
    }
    Cost {
        partial: usd.is_some() && unknown,
        usd,
        unpriced_models: unpriced.into_iter().collect(),
    }
}

/// `$1.23`, `<$0.01` for a positive amount that rounds to nothing, `unknown`, or `>= $1.23 (partial)`.
pub(crate) fn format(cost: &Cost) -> String {
    let Some(usd) = cost.usd else {
        return "unknown".into();
    };
    let amount = if usd > 0.0 && usd < 0.005 {
        "<$0.01".to_string()
    } else {
        format!("${usd:.2}")
    };
    if cost.partial {
        format!(">= {amount} (partial)")
    } else {
        amount
    }
}

/// `format`, with the unpriced models named when there are any.
pub(crate) fn format_with_models(cost: &Cost) -> String {
    let text = format(cost);
    if cost.unpriced_models.is_empty() {
        return text;
    }
    format!(
        "{text}, {} unpriced: {}",
        cost.unpriced_models.len(),
        cost.unpriced_models.join(", ")
    )
}

/// The cost of each model's rows, by model id.
pub(crate) fn by_model(rows: &[UsageRow], table: Table) -> BTreeMap<String, Cost> {
    let models: BTreeSet<&str> = rows.iter().map(|r| r.model.as_str()).collect();
    models
        .into_iter()
        .map(|m| {
            (
                m.to_string(),
                total(rows.iter().filter(|r| r.model == m), table),
            )
        })
        .collect()
}

/// The cost of each stage's rows, by stage name.
pub(crate) fn by_stage(rows: &[UsageRow], table: Table) -> BTreeMap<&'static str, Cost> {
    Stage::ALL
        .into_iter()
        .filter(|s| rows.iter().any(|r| r.stage == *s))
        .map(|s| (s.name(), total(rows.iter().filter(|r| r.stage == s), table)))
        .collect()
}

/// A run's usage as the API shows it: the stored usage plus the estimated cost, per row, Stage, model and
/// in total. Unknown costs are `null`.
pub(crate) fn usage_json(usage: &Usage, table: Table) -> serde_json::Value {
    let mut value = serde_json::to_value(usage).unwrap_or_default();
    if let Some(rows) = value.get_mut("rows").and_then(|r| r.as_array_mut()) {
        for (json, row) in rows.iter_mut().zip(&usage.rows) {
            json["cost_usd"] = serde_json::json!(row_cost(row, table));
        }
    }
    let mut cost = serde_json::to_value(total(&usage.rows, table)).unwrap_or_default();
    cost["by_stage"] = serde_json::json!(by_stage(&usage.rows, table));
    cost["by_model"] = serde_json::json!(by_model(&usage.rows, table));
    value["cost"] = cost;
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TABLE: &[(&str, Price)] = &[("m-priced", price(2.0, 10.0, 0.5, 3.0))];

    fn usage(rows: serde_json::Value) -> Usage {
        serde_json::from_value(json!({"status": "tracked", "collected_at": 0, "rows": rows}))
            .unwrap()
    }

    fn row(stage: &str, model: &str, tokens: serde_json::Value) -> serde_json::Value {
        let mut row = json!({"stage": stage, "model": model});
        row.as_object_mut()
            .unwrap()
            .extend(tokens.as_object().unwrap().clone());
        row
    }

    #[test]
    fn built_in_prices_are_unique_and_not_negative() {
        let ids: BTreeSet<_> = PRICES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), PRICES.len());
        for (id, p) in PRICES {
            assert!(
                [p.input, p.output, p.cache_read, p.cache_write]
                    .iter()
                    .all(|v| *v >= 0.0),
                "{id}"
            );
        }
    }

    #[test]
    fn cost_is_tokens_times_price_per_million() {
        // 1,000,000 * 2 + 100,000 * 10 + 4,000,000 * 0.5 + 200,000 * 3 = 2 + 1 + 2 + 0.6 dollars
        let u = usage(json!([row(
            "build",
            "m-priced",
            json!({"input": 1_000_000, "output": 100_000, "cache_read": 4_000_000, "cache_write": 200_000})
        )]));
        let cost = total(&u.rows, TABLE);
        assert!((cost.usd.unwrap() - 5.6).abs() < 1e-9);
        assert_eq!(format(&cost), "$5.60");
        assert!(!cost.partial);
    }

    #[test]
    fn a_model_without_a_price_is_unknown_never_zero() {
        let u = usage(json!([row(
            "build",
            "m-new",
            json!({"input": 10, "output": 10})
        )]));
        let cost = total(&u.rows, TABLE);
        assert_eq!(cost.usd, None);
        assert_eq!(format(&cost), "unknown");
        assert_eq!(cost.unpriced_models, ["m-new"]);
    }

    #[test]
    fn totals_mixing_priced_and_unpriced_models_are_partial() {
        let u = usage(json!([
            row(
                "build",
                "m-priced",
                json!({"input": 1_000_000, "output": 0})
            ),
            row("check", "m-new", json!({"input": 5, "output": 5})),
        ]));
        let cost = total(&u.rows, TABLE);
        assert!(cost.partial);
        assert_eq!(
            format_with_models(&cost),
            ">= $2.00 (partial), 1 unpriced: m-new"
        );
        let stages = by_stage(&u.rows, TABLE);
        assert_eq!(format(&stages["build"]), "$2.00");
        assert_eq!(format(&stages["check"]), "unknown");
    }

    #[test]
    fn unreported_input_and_output_is_unknown_but_missing_cache_counts_are_costed() {
        let u = usage(json!([
            row("build", "m-priced", json!({"cache_read": 1000})),
            row(
                "build",
                "m-priced",
                json!({"input": 1_000_000, "output": 0})
            ),
        ]));
        assert_eq!(row_cost(&u.rows[0], TABLE), None);
        assert!((row_cost(&u.rows[1], TABLE).unwrap() - 2.0).abs() < 1e-9);
        let only_input = usage(json!([row(
            "build",
            "m-priced",
            json!({"input": 1_000_000})
        )]));
        assert_eq!(format(&total(&only_input.rows, TABLE)), "$2.00");
    }

    #[test]
    fn a_tiny_positive_cost_is_not_shown_as_zero() {
        let cost = Cost {
            usd: Some(0.0004),
            ..Cost::default()
        };
        assert_eq!(format(&cost), "<$0.01");
    }

    #[test]
    fn cost_follows_the_price_table_and_is_not_part_of_the_stored_usage() {
        let u = usage(json!([row(
            "build",
            "m-priced",
            json!({"input": 1_000_000, "output": 0})
        )]));
        let cheaper: Table = &[("m-priced", price(1.0, 1.0, 1.0, 1.0))];
        assert_eq!(format(&total(&u.rows, TABLE)), "$2.00");
        assert_eq!(format(&total(&u.rows, cheaper)), "$1.00");
        assert!(!serde_json::to_string(&u).unwrap().contains("cost"));
    }

    #[test]
    fn api_json_has_null_for_unknown_and_lists_unpriced_models() {
        let u = usage(json!([
            row("build", "m-new", json!({"input": 10, "output": 10})),
            row(
                "build",
                "m-priced",
                json!({"input": 1_000_000, "output": 0})
            ),
        ]));
        let v = usage_json(&u, TABLE);
        assert!(v["rows"][0]["cost_usd"].is_null());
        assert_eq!(v["rows"][1]["cost_usd"], json!(2.0));
        assert_eq!(v["cost"]["partial"], json!(true));
        assert_eq!(v["cost"]["unpriced_models"], json!(["m-new"]));
        assert!(v["cost"]["by_model"]["m-new"]["usd"].is_null());
        assert_eq!(v["cost"]["by_stage"]["build"]["usd"], json!(2.0));
        let none: Usage = serde_json::from_value(
            json!({"status": "not-tracked", "reason": "x", "collected_at": 0}),
        )
        .unwrap();
        let v = usage_json(&none, TABLE);
        assert!(v["cost"]["usd"].is_null());
        assert_eq!(v["cost"]["partial"], json!(false));
    }
}

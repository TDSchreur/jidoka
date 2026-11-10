//! Estimated cost of a run's token usage, computed from a price table at read time and never stored.
//!
//! A token-based estimate at API list prices, not what a ChatGPT/Codex or Claude subscription is charged.
//!
//! A model without a price has an unknown cost, never zero. A total that mixes priced and unpriced models
//! is partial: the sum of what is known, marked as a lower bound.

use crate::{
    run::Stage,
    usage::{Tokens, Usage, UsageRow},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The input, cache-read and output rates of a model, USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rates {
    pub(crate) input: f64,
    pub(crate) cache_read: f64,
    pub(crate) output: f64,
}

/// US dollars per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Price {
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) cache_read: f64,
    /// `None` when the price of a cache write is not known: tokens written to the cache make the cost unknown.
    pub(crate) cache_write: Option<f64>,
    /// Rates for a request above the long-context threshold, when the model has them.
    pub(crate) long: Option<Rates>,
}

const fn price(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        cache_write: Some(cache_write),
        long: None,
    }
}

/// An OpenAI price: the short-context rates, and the long-context column when the page has one.
const fn openai(short: Rates, long: Option<Rates>) -> Price {
    Price {
        input: short.input,
        output: short.output,
        cache_read: short.cache_read,
        cache_write: None,
        long,
    }
}

/// Built-in prices by model id (dots written as dashes), USD per million tokens. This is a token-based
/// estimate at API list prices, not what a ChatGPT/Codex or Claude subscription is charged.
///
/// Claude source: Anthropic's published API prices, as given in the claude-api skill's pricing reference
/// (model table, models guide and prompt-caching guide), read on 2026-11-08. Cache reads are listed
/// explicitly for Fable 5.1 ($0.25), Opus 5.5 and Sonnet 5.5 ($0.20). The claude-opus-4-8 cache prices are
/// derived from Anthropic's documented multipliers: 0.1x input for a cache read, 1.25x input for a
/// 5-minute cache write. Other cache writes are the documented 1.25x input price. Haiku 5.5 is left out
/// (its reference lists tiered input/output prices and no cache prices). Claude prices have no long tier.
///
/// OpenAI source: the Standard tier of https://developers.openai.com/api/docs/pricing (input, cached input,
/// output) and https://developers.openai.com/api/docs/guides/reasoning (reasoning tokens are billed as
/// output), read on 2026-11-10. Assumptions: Codex `input_tokens` include the cached tokens, which Jidoka
/// splits out; a request with more than `LONG_CONTEXT_INPUT_TOKENS` input tokens (cached included) is
/// billed at the long-context rates of the models that list them; Codex does not say which cache retention
/// a cache write used, so OpenAI models have no cache-write price and a row with cache writes is unknown;
/// models the page does not list (such as codex-auto-review, gpt-5-codex, gpt-5.1-codex, gpt-5.2-codex and
/// codex-mini-latest) are unknown; Priority, Flex, Batch and regional uplifts are not applied.
pub(crate) const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5-1", price(10.0, 50.0, 0.25, 12.5)),
    ("claude-opus-5-5", price(4.0, 20.0, 0.20, 5.0)),
    ("claude-sonnet-5-5", price(2.0, 10.0, 0.20, 2.5)),
    ("claude-opus-4-8", price(5.0, 25.0, 0.50, 6.25)),
    (
        "gpt-6-1-terra",
        openai(
            Rates {
                input: 1.0,
                cache_read: 0.05,
                output: 5.0,
            },
            Some(Rates {
                input: 2.0,
                cache_read: 0.10,
                output: 7.5,
            }),
        ),
    ),
    (
        "gpt-6-1-sol",
        openai(
            Rates {
                input: 2.0,
                cache_read: 0.10,
                output: 10.0,
            },
            Some(Rates {
                input: 4.0,
                cache_read: 0.20,
                output: 15.0,
            }),
        ),
    ),
    (
        "gpt-6-astra",
        openai(
            Rates {
                input: 10.0,
                cache_read: 1.0,
                output: 50.0,
            },
            Some(Rates {
                input: 20.0,
                cache_read: 2.0,
                output: 75.0,
            }),
        ),
    ),
    (
        "gpt-6-luna",
        openai(
            Rates {
                input: 0.10,
                cache_read: 0.01,
                output: 0.50,
            },
            Some(Rates {
                input: 0.20,
                cache_read: 0.02,
                output: 0.75,
            }),
        ),
    ),
    (
        "gpt-6-sol",
        openai(
            Rates {
                input: 2.0,
                cache_read: 0.20,
                output: 10.0,
            },
            Some(Rates {
                input: 4.0,
                cache_read: 0.40,
                output: 15.0,
            }),
        ),
    ),
    (
        "gpt-5-6-sol",
        openai(
            Rates {
                input: 4.0,
                cache_read: 0.40,
                output: 20.0,
            },
            Some(Rates {
                input: 8.0,
                cache_read: 0.80,
                output: 30.0,
            }),
        ),
    ),
    (
        "gpt-5-6-terra",
        openai(
            Rates {
                input: 2.0,
                cache_read: 0.20,
                output: 12.0,
            },
            Some(Rates {
                input: 4.0,
                cache_read: 0.40,
                output: 18.0,
            }),
        ),
    ),
    (
        "gpt-5-6-luna",
        openai(
            Rates {
                input: 0.20,
                cache_read: 0.02,
                output: 1.20,
            },
            Some(Rates {
                input: 0.40,
                cache_read: 0.04,
                output: 1.80,
            }),
        ),
    ),
    (
        "gpt-5-5",
        openai(
            Rates {
                input: 5.0,
                cache_read: 0.50,
                output: 30.0,
            },
            Some(Rates {
                input: 10.0,
                cache_read: 1.0,
                output: 45.0,
            }),
        ),
    ),
    (
        "gpt-5-4",
        openai(
            Rates {
                input: 2.5,
                cache_read: 0.25,
                output: 15.0,
            },
            Some(Rates {
                input: 5.0,
                cache_read: 0.50,
                output: 22.5,
            }),
        ),
    ),
    (
        "gpt-5-4-mini",
        openai(
            Rates {
                input: 0.75,
                cache_read: 0.075,
                output: 4.5,
            },
            None,
        ),
    ),
    (
        "gpt-5-3-codex",
        openai(
            Rates {
                input: 1.75,
                cache_read: 0.175,
                output: 14.0,
            },
            None,
        ),
    ),
    (
        "gpt-5-2",
        openai(
            Rates {
                input: 1.75,
                cache_read: 0.175,
                output: 14.0,
            },
            None,
        ),
    ),
    (
        "gpt-5-1",
        openai(
            Rates {
                input: 1.25,
                cache_read: 0.125,
                output: 10.0,
            },
            None,
        ),
    ),
    (
        "gpt-5",
        openai(
            Rates {
                input: 1.25,
                cache_read: 0.125,
                output: 10.0,
            },
            None,
        ),
    ),
];

pub(crate) type Table<'a> = &'a [(&'a str, Price)];

/// Claude Code logs `claude-sonnet-5-5` where Copilot logs `claude-sonnet-5.5`: the same model, so a dot
/// and a dash match.
fn lookup(table: Table, model: &str) -> Option<Price> {
    let model = model.replace('.', "-");
    table.iter().find(|(id, _)| *id == model).map(|(_, p)| *p)
}

fn minus(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    a.map(|a| a.saturating_sub(b.unwrap_or(0)))
}

/// Cost of `tokens` in dollars times a million: each count times its per-million price, `rates` for input,
/// cache reads and output and `cache_write` for cache writes (uncounted fields add nothing).
fn cost_at(tokens: &Tokens, rates: &Rates, cache_write: f64) -> f64 {
    let part = |count: Option<u64>, per_million: f64| count.unwrap_or(0) as f64 * per_million;
    part(tokens.input, rates.input)
        + part(tokens.output, rates.output)
        + part(tokens.cache_read, rates.cache_read)
        + part(tokens.cache_write, cache_write)
}

/// The cost of one set of tokens in dollars; `long_context` is the part of them from requests above the
/// long-context threshold, priced at the long rates when the model has them. `None` when neither input nor
/// output was reported, or when cache writes were reported and the model has no cache-write price; counts
/// that were not reported otherwise add nothing.
pub(crate) fn tokens_cost(tokens: &Tokens, long_context: &Tokens, price: &Price) -> Option<f64> {
    if tokens.input.is_none() && tokens.output.is_none() {
        return None;
    }
    let written = tokens.cache_write.unwrap_or(0);
    let write_price = match price.cache_write {
        Some(p) => p,
        None if written > 0 => return None,
        None => 0.0,
    };
    let (short, long) = match &price.long {
        Some(_) => (
            Tokens {
                input: minus(tokens.input, long_context.input),
                cache_read: minus(tokens.cache_read, long_context.cache_read),
                cache_write: minus(tokens.cache_write, long_context.cache_write),
                output: minus(tokens.output, long_context.output),
                reasoning: None,
            },
            *long_context,
        ),
        None => (*tokens, Tokens::default()),
    };
    let short_rates = Rates {
        input: price.input,
        cache_read: price.cache_read,
        output: price.output,
    };
    let mut total = cost_at(&short, &short_rates, write_price);
    if let Some(rates) = &price.long {
        total += cost_at(&long, rates, write_price);
    }
    Some(total / 1e6)
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

/// The cost of a row, `None` when its model has no price, it reported no input and output, or it has cache
/// writes the model's price does not cover.
pub(crate) fn row_cost(row: &UsageRow, table: Table) -> Option<f64> {
    tokens_cost(&row.tokens, &row.long_context, &lookup(table, &row.model)?)
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

/// Below this many dollars, `{:.2}` formatting would show `$0.00`.
const HALF_A_CENT: f64 = 0.005;

/// `$1.23`, `<$0.01` for a positive amount that rounds to nothing, or `unknown` for `None`.
pub(crate) fn format_usd(usd: Option<f64>) -> String {
    match usd {
        None => "unknown".into(),
        Some(usd) if usd > 0.0 && usd < HALF_A_CENT => "<$0.01".into(),
        Some(usd) => format!("${usd:.2}"),
    }
}

/// `$1.23`, `<$0.01` for a positive amount that rounds to nothing, `unknown`, or `>= $1.23 (partial)`.
pub(crate) fn format(cost: &Cost) -> String {
    let amount = format_usd(cost.usd);
    if cost.usd.is_some() && cost.partial {
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
pub(crate) fn usage_json(usage: &Usage, table: Table) -> serde_json::Result<serde_json::Value> {
    let mut value = serde_json::to_value(usage)?;
    if let Some(rows) = value.get_mut("rows").and_then(|r| r.as_array_mut()) {
        for (json, row) in rows.iter_mut().zip(&usage.rows) {
            json["cost_usd"] = serde_json::json!(row_cost(row, table));
        }
    }
    let mut cost = serde_json::to_value(total(&usage.rows, table))?;
    cost["by_stage"] = serde_json::json!(by_stage(&usage.rows, table));
    cost["by_model"] = serde_json::json!(by_model(&usage.rows, table));
    value["cost"] = cost;
    Ok(value)
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
                [
                    p.input,
                    p.output,
                    p.cache_read,
                    p.cache_write.unwrap_or(0.0)
                ]
                .iter()
                .all(|v| *v >= 0.0),
                "{id}"
            );
        }
    }

    #[test]
    fn a_dotted_model_id_is_priced_like_its_dashed_form() {
        let table: Table = &[("claude-m-5-5", price(2.0, 10.0, 0.5, 3.0))];
        assert!(lookup(table, "claude-m-5.5").is_some());
        assert!(lookup(table, "claude-m-5-5").is_some());
        assert!(lookup(table, "claude-m-5.6").is_none());
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
        assert_eq!(stages["check"].unpriced_models, ["m-new"]);
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
        let v = usage_json(&u, TABLE).unwrap();
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
        let v = usage_json(&none, TABLE).unwrap();
        assert!(v["cost"]["usd"].is_null());
        assert_eq!(v["cost"]["partial"], json!(false));
    }
}

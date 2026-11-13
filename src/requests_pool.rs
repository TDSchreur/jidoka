//! `jidoka run requests --all`: the model requests of many runs pooled, with how they spread over the runs.

use crate::{
    inspect::shown,
    request_logs::Category,
    requests::{Counts, RunRequests, analyze, cells},
    run::{Run, RunStatus},
    stats::{number, table_with},
    usage::UsageStatus,
    usage_logs::Roots,
};
use serde::Serialize;
use std::{collections::BTreeMap, time::Instant};

/// How complete a run is, from the run's own data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Class {
    /// The run is done and its token usage is tracked in full.
    Completed,
    /// The run is done, but its token usage is partial, missing, or has stages with no data.
    Partial,
    /// The run is still going.
    Open,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Completed => "completed",
            Class::Partial => "partial",
            Class::Open => "open",
        }
    }
}

fn class_of(run: &Run) -> Class {
    if !matches!(run.status, RunStatus::Done) {
        Class::Open
    } else if run
        .usage
        .as_ref()
        .is_none_or(|usage| usage.status != UsageStatus::Tracked || !usage.no_data.is_empty())
    {
        Class::Partial
    } else {
        Class::Completed
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PoolOptions {
    /// Also pool runs that are partial or still open.
    pub(crate) include_partial: bool,
    /// Runs to leave out, such as development runs.
    pub(crate) exclude: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct IncludedRun {
    pub(crate) run: String,
    pub(crate) title: String,
    pub(crate) class: Class,
    pub(crate) requests: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct LeftOut {
    pub(crate) run: String,
    pub(crate) title: String,
    pub(crate) reason: String,
}

/// One category over the pooled runs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PooledCategory {
    pub(crate) category: Category,
    /// Requests per run, over every pooled run, so a run without the category counts as zero.
    pub(crate) median: f64,
    pub(crate) p25: f64,
    pub(crate) p75: f64,
    pub(crate) min: u64,
    pub(crate) max: u64,
    /// The category's part of all pooled requests, from 0 to 1.
    pub(crate) request_share: f64,
    /// The category's part of all pooled fresh tokens, from 0 to 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fresh_share: Option<f64>,
    #[serde(flatten)]
    pub(crate) counts: Counts,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PooledRequests {
    pub(crate) included: Vec<IncludedRun>,
    pub(crate) left_out: Vec<LeftOut>,
    pub(crate) categories: Vec<PooledCategory>,
    pub(crate) total: Counts,
    pub(crate) subagents: Counts,
    pub(crate) notes: Vec<String>,
}

/// `[--json] [--include-partial] [--exclude <id>]...` in any order: the pool options and whether to print JSON.
pub(crate) fn parse_options(options: &[&str]) -> Result<(PoolOptions, bool), String> {
    let (mut pool, mut json) = (PoolOptions::default(), false);
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--json" if !json => json = true,
            "--include-partial" if !pool.include_partial => pool.include_partial = true,
            "--exclude" => {
                let id = args.next().copied().unwrap_or("");
                if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("--exclude needs a run id".into());
                }
                pool.exclude.push(id.to_string());
            }
            other => return Err(format!("Unknown or repeated option {}", shown(other))),
        }
    }
    Ok((pool, json))
}

/// The value at fraction `p` of the sorted `values`, interpolating between the two nearest ranks.
fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = p * (sorted.len() - 1) as f64;
    let (low, high) = (rank.floor() as usize, rank.ceil() as usize);
    let (a, b) = (sorted[low] as f64, sorted[high] as f64);
    a + (b - a) * (rank - low as f64)
}

#[expect(
    clippy::too_many_lines,
    reason = "does several steps in one function; deepening candidate"
)]
pub(crate) fn pool(
    runs: &[Run],
    roots: &Roots,
    now: u64,
    deadline: Instant,
    options: &PoolOptions,
) -> PooledRequests {
    let mut out = PooledRequests {
        included: Vec::new(),
        left_out: Vec::new(),
        categories: Vec::new(),
        total: Counts::default(),
        subagents: Counts::default(),
        notes: Vec::new(),
    };
    for id in &options.exclude {
        if !runs.iter().any(|run| run.id == *id) {
            out.notes
                .push(format!("--exclude {} is not a known run id", shown(id)));
        }
    }
    let mut ordered: Vec<&Run> = runs.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let mut results: Vec<RunRequests> = Vec::new();
    for run in ordered {
        let left_out = |reason: String| LeftOut {
            run: run.id.clone(),
            title: run.title.clone(),
            reason,
        };
        let class = class_of(run);
        if options.exclude.contains(&run.id) {
            out.left_out
                .push(left_out("left out with --exclude".into()));
            continue;
        }
        if class != Class::Completed && !options.include_partial {
            out.left_out.push(left_out(match class {
                Class::Open => "the run is still open (--include-partial pools it)".into(),
                Class::Partial | Class::Completed => {
                    "its token usage is partial or missing (--include-partial pools it)".into()
                }
            }));
            continue;
        }
        let result = analyze(run, runs, roots, now, deadline);
        // A reading that was cut short undercounts every run after it: say so once.
        for note in &result.notes {
            if note.starts_with("reading the session logs") && !out.notes.contains(note) {
                out.notes.push(note.clone());
            }
        }
        if result.total.requests == 0 {
            out.left_out
                .push(left_out("no requests were found for it".into()));
            continue;
        }
        out.included.push(IncludedRun {
            run: run.id.clone(),
            title: run.title.clone(),
            class,
            requests: result.total.requests,
        });
        results.push(result);
    }

    // Requests per run for every category, with zero for a run that has none.
    let mut per_run: BTreeMap<Category, Vec<u64>> = BTreeMap::new();
    let mut counts: BTreeMap<Category, Counts> = BTreeMap::new();
    for (index, result) in results.iter().enumerate() {
        out.total.merge(&result.total);
        out.subagents.merge(&result.subagents);
        for row in &result.categories {
            let values = per_run
                .entry(row.category)
                .or_insert_with(|| vec![0; results.len()]);
            values[index] += row.counts.requests;
            counts.entry(row.category).or_default().merge(&row.counts);
        }
    }
    let total_fresh = out.total.tokens.fresh();
    for (category, mut values) in per_run {
        values.sort_unstable();
        let counts = counts.remove(&category).unwrap_or_default();
        out.categories.push(PooledCategory {
            category,
            median: percentile(&values, 0.5),
            p25: percentile(&values, 0.25),
            p75: percentile(&values, 0.75),
            min: values.first().copied().unwrap_or(0),
            max: values.last().copied().unwrap_or(0),
            request_share: counts.requests as f64 / out.total.requests.max(1) as f64,
            fresh_share: match (counts.tokens.fresh(), total_fresh) {
                (Some(part), Some(whole)) if whole > 0 => Some(part as f64 / whole as f64),
                _ => None,
            },
            counts,
        });
    }

    if out.subagents.requests > 0 {
        out.notes.push(format!(
            "Subagent requests are included: {} of the {} requests",
            out.subagents.requests, out.total.requests
        ));
    }
    let (coordinator_only, sessions) = results.iter().fold((0, 0), |(only, all), r| {
        (
            only + r.coordinator_only_sessions.len(),
            all + r.sessions.len(),
        )
    });
    if coordinator_only > 0 {
        out.notes.push(format!(
            "{coordinator_only} of {sessions} sessions have no subagent logs, so their figures are coordinator-only"
        ));
    }
    let ambiguous: u64 = results.iter().map(|r| r.ambiguous.requests).sum();
    if ambiguous > 0 {
        out.notes.push(format!(
            "{ambiguous} requests were made in sessions shared with another run at the same time and are in no run"
        ));
    }
    if out
        .categories
        .iter()
        .any(|row| row.category == Category::NotCategorized)
    {
        out.notes.push(
            "Some Copilot requests have no matching assistant message in the log, so they are not categorized and have no tool output size; the tool output totals leave them out".into(),
        );
    }
    if out.included.len() < 10 {
        out.notes.push(format!(
            "{} runs is a small sample; the spread says more than the median",
            out.included.len()
        ));
    }
    out
}

/// `2`, or `2.5`.
fn amount(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn percent(part: f64) -> String {
    format!("{:.1}%", part * 100.0)
}

const EXPLANATION: &str = "Requests per run are counted over every pooled run, so a run without the category counts as zero. Percentiles interpolate between the nearest runs. The fresh share is the category's part of input excluding cache reads, plus cache writes and output; the token columns are never added together. n/a means the log did not report it.";

pub(crate) fn render_pool(result: &PooledRequests) -> String {
    let runs = result.included.len();
    let mut out = format!(
        "Model requests over {runs} {}\n\n{EXPLANATION}\n\n",
        if runs == 1 { "run" } else { "runs" }
    );
    if runs == 0 {
        out.push_str("  No runs were pooled.\n");
    } else {
        let spread: Vec<Vec<String>> = result
            .categories
            .iter()
            .map(|row| {
                vec![
                    row.category.label().to_string(),
                    amount(row.median),
                    amount(row.p25),
                    amount(row.p75),
                    row.min.to_string(),
                    row.max.to_string(),
                ]
            })
            .collect();
        out.push_str("Requests per run\n");
        out.push_str(&table_with(
            &spread,
            &["Category", "Median", "P25", "P75", "Min", "Max"],
            1,
        ));
        let total = result.total.requests;
        let mut pooled: Vec<Vec<String>> = result
            .categories
            .iter()
            .map(|row| {
                let mut cells = cells(row.category.label(), &row.counts, total);
                cells.insert(3, row.fresh_share.map_or_else(|| "n/a".into(), percent));
                cells
            })
            .collect();
        let mut all = cells("total", &result.total, total);
        all.insert(3, "100.0%".into());
        pooled.push(all);
        out.push_str("\nAll pooled requests\n");
        out.push_str(&table_with(
            &pooled,
            &[
                "Category",
                "Requests",
                "Share",
                "Fresh share",
                "Input",
                "Cache read",
                "Cache write",
                "Output",
                "Tool output",
                "Largest",
            ],
            1,
        ));
        out.push_str("\nRuns pooled\n");
        for run in &result.included {
            out.push_str(&format!(
                "  {}  {}  {}  {} requests\n",
                shown(&run.run),
                run.class.label(),
                shown(&run.title),
                number(Some(run.requests))
            ));
        }
    }
    if !result.left_out.is_empty() {
        out.push_str("\nLeft out\n");
        for run in &result.left_out {
            out.push_str(&format!(
                "  {}  {}: {}\n",
                shown(&run.run),
                shown(&run.title),
                run.reason
            ));
        }
    }
    if !result.notes.is_empty() {
        out.push_str("\nWhat these figures do not cover\n");
        for note in &result.notes {
            out.push_str(&format!("  - {}\n", shown(note)));
        }
    }
    out
}

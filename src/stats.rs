//! `jidoka run stats`: token usage per stage, model and ticket, in plain text.

use crate::{
    cost::{self, PRICES},
    inspect::shown,
    run::{Run, Stage},
    usage::{Tokens, Usage, UsageRow, UsageStatus, format_timestamp},
};
use std::collections::BTreeMap;

const EXPLANATION: &str = "Fresh tokens are input excluding cache reads, plus cache writes and output. Cache reads are shown beside them, not inside. Reasoning is part of output and is not added again. n/a means the log did not report it; it is never zero.\nEst. cost is a token-based estimate at API list prices (USD per million tokens), not what a subscription is charged; a model without a price, or OpenAI usage with cache writes, is unknown, never zero, and a total that includes one is marked partial.";

/// `12,345`, or `n/a` for a count that was not reported.
pub(crate) fn number(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "n/a".into();
    };
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn bucket(row: &UsageRow) -> String {
    match (&row.ticket, row.rework, row.stage) {
        (Some(reference), _, _) => shown(reference),
        (None, true, _) => "rework".into(),
        (None, false, Stage::Build) => "no ticket".into(),
        _ => String::new(),
    }
}

fn table(rows: &[Vec<String>], header: &[&str]) -> String {
    table_with(rows, header, 3)
}

/// A table whose first `text_columns` columns are left-aligned and the rest right-aligned.
pub(crate) fn table_with(rows: &[Vec<String>], header: &[&str], text_columns: usize) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let mut out = String::from("  ");
        for (index, cell) in cells.iter().enumerate() {
            // Text columns are left-aligned, numbers right-aligned.
            if index < text_columns {
                out.push_str(&format!("{cell:<width$}  ", width = widths[index]));
            } else {
                out.push_str(&format!("{cell:>width$}  ", width = widths[index]));
            }
        }
        out.trim_end().to_string() + "\n"
    };
    let mut out = line(header.to_vec());
    for row in rows {
        out.push_str(&line(row.iter().map(String::as_str).collect()));
    }
    out
}

fn token_cells(tokens: &Tokens, subagents: &Tokens, cost: String) -> Vec<String> {
    vec![
        number(tokens.fresh()),
        number(tokens.cache_read),
        number(tokens.input),
        number(tokens.cache_write),
        number(tokens.output),
        number(tokens.reasoning),
        if subagents.is_empty() {
            "-".into()
        } else {
            number(subagents.fresh())
        },
        cost,
    ]
}

const HEADER: [&str; 11] = [
    "Stage",
    "Model",
    "Ticket",
    "Fresh",
    "Cache read",
    "Input",
    "Cache write",
    "Output",
    "Reasoning",
    "Subagents",
    "Est. cost",
];

/// The totals of the rows per model.
fn per_model(rows: &[UsageRow]) -> BTreeMap<String, (Tokens, Tokens)> {
    let mut models: BTreeMap<String, (Tokens, Tokens)> = BTreeMap::new();
    for row in rows {
        let entry = models.entry(row.model.clone()).or_default();
        entry.0.add(&row.tokens);
        entry.1.add(&row.subagents);
    }
    models
}

fn model_rows(rows: &[UsageRow]) -> Vec<Vec<String>> {
    let costs = cost::by_model(rows, PRICES);
    per_model(rows)
        .into_iter()
        .map(|(model, (tokens, subagents))| {
            let mut cells = vec![String::new(), shown(&model), String::new()];
            cells.extend(token_cells(
                &tokens,
                &subagents,
                cost::format(&costs[&model]),
            ));
            cells
        })
        .collect()
}

fn status_line(usage: &Usage) -> String {
    match usage.status {
        UsageStatus::Tracked => "tracked".to_string(),
        UsageStatus::Partial => "partial: see the coverage notes below".to_string(),
        UsageStatus::NotTracked => {
            format!(
                "not tracked: {}",
                shown(usage.reason.as_deref().unwrap_or("no reason recorded"))
            )
        }
    }
}

pub(crate) fn render_run(run: &Run) -> String {
    let mut out = format!("Run {}: {}\n", run.id, shown(&run.title));
    let Some(usage) = &run.usage else {
        out.push_str("Tokens: not tracked: usage has not been collected for this run yet (it starts at the next stage change, or the run predates usage tracking)\n");
        return out;
    };
    out.push_str(&format!("Tokens: {}\n", status_line(usage)));
    out.push_str(&format!(
        "Collected {} from {} session(s)\n",
        format_timestamp(usage.collected_at),
        usage.sessions.len()
    ));
    if !usage.rows.is_empty() {
        out.push_str(&format!("\n{EXPLANATION}\n\n"));
        let rows: Vec<Vec<String>> = usage
            .rows
            .iter()
            .map(|row| {
                let mut cells = vec![row.stage.name().to_string(), shown(&row.model), bucket(row)];
                let row_cost = cost::format_usd(cost::row_cost(row, PRICES));
                cells.extend(token_cells(&row.tokens, &row.subagents, row_cost));
                cells
            })
            .collect();
        out.push_str(&table(&rows, &HEADER));
        out.push_str("\nPer model\n");
        out.push_str(&table(&model_rows(&usage.rows), &HEADER));
        out.push_str("\nEstimated cost per stage\n");
        for (stage, stage_cost) in cost::by_stage(&usage.rows, PRICES) {
            out.push_str(&format!(
                "  {stage}: {}\n",
                cost::format_with_models(&stage_cost)
            ));
        }
        out.push_str(&format!(
            "\nTotal fresh tokens: {}\n",
            number(usage.fresh())
        ));
        out.push_str(&format!(
            "Total estimated cost: {}\n",
            cost::format_with_models(&cost::total(&usage.rows, PRICES))
        ));
    }
    for stage in &usage.no_data {
        out.push_str(&format!("{}: no data\n", stage.name()));
    }
    if !usage.unattributed.is_empty() {
        out.push_str(&format!(
            "Not assigned (shared with another run): {} fresh tokens\n",
            number(usage.unattributed.fresh())
        ));
    }
    if !usage.notes.is_empty() {
        out.push_str("\nCoverage notes\n");
        for note in &usage.notes {
            out.push_str(&format!("  - {}\n", shown(note)));
        }
    }
    out
}

pub(crate) fn render_all(runs: &[Run]) -> String {
    let mut out = String::from("Token usage over all runs\n\n");
    out.push_str(&format!("{EXPLANATION}\n\n"));
    let mut all = Vec::new();
    let mut left_out: Vec<String> = Vec::new();
    let mut partial: Vec<&str> = Vec::new();
    let mut lines: Vec<Vec<String>> = Vec::new();
    for run in runs {
        let (state, fresh) = match &run.usage {
            Some(usage) if usage.status != UsageStatus::NotTracked => {
                all.extend(usage.rows.iter().cloned());
                if usage.status == UsageStatus::Partial {
                    partial.push(&run.id);
                }
                (
                    if usage.status == UsageStatus::Partial {
                        "partial"
                    } else {
                        "tracked"
                    },
                    number(usage.fresh()),
                )
            }
            Some(usage) => {
                left_out.push(format!(
                    "{} ({}): {}",
                    run.id,
                    shown(&run.title),
                    shown(usage.reason.as_deref().unwrap_or("no reason recorded"))
                ));
                ("not tracked", "n/a".to_string())
            }
            None => {
                left_out.push(format!(
                    "{} ({}): usage was never collected",
                    run.id,
                    shown(&run.title)
                ));
                ("not tracked", "n/a".to_string())
            }
        };
        lines.push(vec![
            run.id.clone(),
            state.to_string(),
            fresh,
            shown(&run.title),
        ]);
    }
    if lines.is_empty() {
        out.push_str("There are no runs.\n");
        return out;
    }
    let id_width = lines.iter().map(|l| l[0].len()).max().unwrap_or(0);
    let state_width = lines.iter().map(|l| l[1].len()).max().unwrap_or(0);
    let fresh_width = lines.iter().map(|l| l[2].len()).max().unwrap_or(0).max(5);
    out.push_str(&format!(
        "  {:<id_width$}  {:<state_width$}  {:>fresh_width$}  Title\n",
        "Run", "Usage", "Fresh"
    ));
    for line in &lines {
        out.push_str(&format!(
            "  {:<id_width$}  {:<state_width$}  {:>fresh_width$}  {}\n",
            line[0], line[1], line[2], line[3]
        ));
    }
    out.push_str("\nPer model, over the runs whose usage is tracked or partial\n");
    let models = model_rows(&all);
    if models.is_empty() {
        out.push_str("  no model usage is tracked\n");
    } else {
        out.push_str(&table(&models, &HEADER));
        out.push_str(&format!(
            "\nTotal estimated cost: {}\n",
            cost::format_with_models(&cost::total(&all, PRICES))
        ));
    }
    if !partial.is_empty() {
        out.push_str(&format!("\nPartial coverage: {} (run `jidoka run stats <id>` for what is missing). Their totals may be too low.\n", partial.join(", ")));
    }
    if !left_out.is_empty() {
        out.push_str("\nLeft out, because their usage is not tracked (not counted as zero):\n");
        for line in &left_out {
            out.push_str(&format!("  - {line}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(id: &str, title: &str, usage: serde_json::Value) -> Run {
        serde_json::from_value(json!({"id": id, "title": title, "project": "/p", "stage": "finish", "status": "done", "usage": usage})).unwrap()
    }

    fn priced() -> serde_json::Value {
        json!({"status": "tracked", "collected_at": 0, "rows": [
            {"stage": "build", "model": "claude-opus-5-5", "input": 1_000_000, "cache_read": 10_000_000, "output": 100_000},
            {"stage": "check", "model": "claude-opus-5-5", "input": 500_000, "output": 0},
            {"stage": "check", "model": "gpt-test", "input": 5, "output": 5}
        ]})
    }

    #[test]
    fn stats_show_estimated_cost_per_row_model_stage_and_run_with_unpriced_models_unknown() {
        // Opus 5.5: $4 in, $20 out, $0.20 cache read per million tokens.
        // build: 1M*4 + 0.1M*20 + 10M*0.2 = 4 + 2 + 2 = $8.00; check: 0.5M*4 = $2.00.
        let text = render_run(&run("1", "Task", priced()));
        let line = |needle: &str| {
            text.lines()
                .find(|l| l.contains(needle))
                .unwrap()
                .to_string()
        };
        assert!(line("build").trim_end().ends_with("$8.00"), "{text}");
        assert!(line("gpt-test").trim_end().ends_with("unknown"), "{text}");
        assert!(text.contains("  build: $8.00\n"), "{text}");
        assert!(
            text.contains("  check: >= $2.00 (partial), 1 unpriced: gpt-test\n"),
            "{text}"
        );
        assert!(
            text.contains("Total estimated cost: >= $10.00 (partial), 1 unpriced: gpt-test"),
            "{text}"
        );
        assert!(!text.contains("$0.00"), "{text}");
        // Cache read sits right after Fresh in the per-model table.
        let header = text
            .lines()
            .find(|l| l.trim_start().starts_with("Stage"))
            .unwrap();
        assert!(header.find("Fresh") < header.find("Cache read"), "{header}");
        assert!(header.find("Cache read") < header.find("Input"), "{header}");
    }

    #[test]
    fn a_run_without_tracked_usage_says_so_and_never_shows_zero() {
        let none = render_run(&run(
            "1",
            "Task",
            json!({"status": "not-tracked", "reason": "no session log names this run", "collected_at": 0}),
        ));
        assert!(
            none.contains("Tokens: not tracked: no session log names this run"),
            "{none}"
        );
        assert!(!none.contains("Total fresh"), "{none}");
        let never: Run = serde_json::from_value(json!({"id": "2", "title": "Old", "project": "/p", "stage": "build", "status": "running"})).unwrap();
        let text = render_run(&never);
        assert!(
            text.contains("not tracked: usage has not been collected"),
            "{text}"
        );
        assert!(!text.lines().any(|l| l.trim() == "0"), "{text}");
    }

    #[test]
    fn stats_for_all_runs_total_per_model_and_name_the_runs_left_out() {
        let runs = [
            run("10", "Tracked run", json!({"status": "tracked", "collected_at": 0, "rows": [{"stage": "build", "model": "m-1", "input": 5, "output": 5}]})),
            run("11", "Partial run", json!({"status": "partial", "collected_at": 0, "no_data": ["define"], "rows": [{"stage": "build", "model": "m-1", "input": 1, "output": 1}]})),
            run("12", "Lost run", json!({"status": "not-tracked", "reason": "logs were not found", "collected_at": 0})),
            serde_json::from_value(json!({"id": "13", "title": "Never", "project": "/p", "stage": "build", "status": "running"})).unwrap(),
        ];
        let text = render_all(&runs);
        let per_model = text.split("Per model").nth(1).unwrap();
        assert!(
            per_model
                .lines()
                .any(|l| l.contains("m-1") && l.contains("12")),
            "5+5+1+1 fresh: {per_model}"
        );
        assert!(text.contains("Partial coverage: 11"), "{text}");
        assert!(
            text.contains("12 (Lost run): logs were not found"),
            "{text}"
        );
        assert!(
            text.contains("13 (Never): usage was never collected"),
            "{text}"
        );
        assert!(text.contains("not counted as zero"), "{text}");
        assert!(render_all(&[]).contains("There are no runs."));
    }
}

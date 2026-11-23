//! The reports `pr open` and `pr sync` send, against the update the server already takes.

use super::*;
use crate::run::parse_update;

/// The JSON body of an update, as `POST /api/runs/{id}` receives it.
fn body(update: &UpdateRun) -> serde_json::Value {
    serde_json::to_value(update).unwrap()
}

#[test]
fn each_report_sends_the_update_the_cli_options_used_to_send() {
    let url = "https://github.com/o/r/pull/7";
    let base = || "develop".to_string();
    let cases = [
        (
            PrReport::Opened {
                pull_request: url.into(),
            },
            vec!["--pr", url, "--activity", "Opened pull request"],
        ),
        (
            PrReport::BaseMerged { base: base() },
            vec!["--activity", "Merged develop into the branch"],
        ),
        (
            PrReport::BaseNeedsVerification { base: base() },
            vec!["--activity", "Merged develop: verification needed"],
        ),
        (
            PrReport::BaseConflicts { base: base() },
            vec!["--activity", "Merging develop: conflicts to resolve"],
        ),
    ];
    for (report, options) in cases {
        let update = report.update();
        update.validate().unwrap();
        assert_eq!(
            body(&update),
            body(&parse_update(&options).unwrap()),
            "{report:?}"
        );
    }
}

#[test]
fn an_opened_report_with_an_unusable_url_is_refused_before_it_is_sent() {
    let update = PrReport::Opened {
        pull_request: "not a url".into(),
    }
    .update();
    assert!(update.validate().is_err());
}

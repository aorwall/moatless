use moatless_api_schemas::decisions::{DecisionBand, DecisionUsage, OptionProbability};

use super::*;

#[derive(clap::Parser)]
struct TestCli {
    #[command(subcommand)]
    action: DecideAction,
}

fn parse(argv: &[&str]) -> Result<PickArgs, clap::Error> {
    let mut full = vec!["moat", "pick", "Bug or feature?"];
    full.extend_from_slice(argv);
    <TestCli as clap::Parser>::try_parse_from(full).map(|cli| {
        let DecideAction::Pick(args) = cli.action;
        *args
    })
}

fn option(name: &str, description: Option<&str>) -> DecisionOption {
    DecisionOption {
        name: name.to_string(),
        description: description.map(str::to_string),
    }
}

#[test]
fn options_come_from_a_comma_list_and_repeated_described_flags() {
    let args = parse(&[
        "--options",
        "bug,feature",
        "--option",
        "docs=Only documentation changes",
        "--option",
        "chore",
        "--option",
        "ops=deploy=infra",
    ])
    .expect("parses");

    assert_eq!(
        parse_options(&args.options, &args.option),
        vec![
            option("bug", None),
            option("feature", None),
            option("docs", Some("Only documentation changes")),
            option("chore", None),
            option("ops", Some("deploy=infra")),
        ]
    );
}

#[test]
fn options_are_required_and_state_sources_are_exclusive() {
    assert!(parse(&["--state", "text"]).is_err());
    assert!(parse(&["--options", "a,b", "--state", "x", "--state-file", "f"]).is_err());
    let args = parse(&["--option", "a", "--option", "b", "--each"]).expect("parses");
    assert!(args.each);
    assert!(args.state.is_none());
}

#[test]
fn a_batch_reads_as_jsonl_or_an_array_of_strings_or_objects() {
    let jsonl =
        "{\"id\": \"SP-1\", \"state\": \"one\"}\n\n\"two\"\n{\"id\": 7, \"state\": \"three\"}\n";
    let array = r#"[{"id": "SP-1", "state": "one"}, "two", {"id": 7, "state": "three"}]"#;
    let expected = vec![
        BatchItem {
            id: "SP-1".to_string(),
            state: "one".to_string(),
        },
        BatchItem {
            id: "2".to_string(),
            state: "two".to_string(),
        },
        BatchItem {
            id: "7".to_string(),
            state: "three".to_string(),
        },
    ];

    assert_eq!(parse_batch(jsonl).expect("jsonl"), expected);
    assert_eq!(parse_batch(array).expect("array"), expected);
}

#[test]
fn a_bad_batch_names_the_line_or_item() {
    let cases = [
        ("\"ok\"\nnot json\n", "line 2"),
        (
            "\"ok\"\n{\"id\": \"x\"}\n",
            "line 2: expected a string or an object",
        ),
        (r#"["ok", 5]"#, "item 2: expected a string or an object"),
        ("\n  \n", "no items"),
    ];
    for (input, expected) in cases {
        let error = parse_batch(input).expect_err(input);
        assert!(error.contains(expected), "{error:?} lacks {expected:?}");
    }
}

fn response(confidence: Option<f64>, band: DecisionBand) -> PickDecisionResponse {
    PickDecisionResponse {
        choice: "moatless".to_string(),
        confidence,
        band,
        probabilities: vec![
            OptionProbability {
                name: "moatless".to_string(),
                probability: 0.95,
            },
            OptionProbability {
                name: "soap-infra".to_string(),
                probability: 0.04,
            },
            OptionProbability {
                name: "soap-platform-gitops".to_string(),
                probability: 0.01,
            },
        ],
        provider: "jev".to_string(),
        model: "jev-1.13.0".to_string(),
        usage: DecisionUsage {
            input_tokens: 812,
            output_tokens: 40,
        },
    }
}

#[test]
fn a_single_decision_prints_choice_alternatives_and_usage() {
    assert_eq!(
        summary(&response(Some(0.93), DecisionBand::Act)),
        "choice: moatless (0.93, act)\n\
         alternatives: soap-infra 0.04, soap-platform-gitops 0.01\n\
         usage: jev/jev-1.13.0 · 812 input tokens"
    );
    let mut bare = response(None, DecisionBand::Escalate);
    bare.probabilities.clear();
    assert_eq!(
        summary(&bare),
        "choice: moatless (escalate)\nusage: jev/jev-1.13.0 · 812 input tokens"
    );
}

#[test]
fn a_batch_row_is_tab_separated_and_an_error_row_carries_the_message() {
    assert_eq!(
        tsv_row("SP-1", &Ok(response(Some(0.93), DecisionBand::Act))),
        "SP-1\tmoatless\t0.93\tact\tsoap-infra"
    );
    let error = ApiError::Api {
        status: 400,
        body: r#"{"error": "state must not be empty"}"#.to_string(),
    };
    assert_eq!(
        tsv_row("a\tb", &Err(error)),
        "a b\t-\t-\terror\t400: state must not be empty"
    );
}

//! The credentialed evaluation, run through `task test:moat-cli:eval`.

use super::*;

#[tokio::test]
#[ignore = "requires real Claude/Codex CLIs and credentials (MOAT_CLI_EVAL=1)"]
async fn evaluate_agents_using_moat_cli() {
    if std::env::var(EVAL_GATE).as_deref() != Ok("1") {
        eprintln!("skipping: set {EVAL_GATE}=1 or run `task test:moat-cli:eval`");
        return;
    }

    let settings = EvalSettings::from_env().expect("invalid moat CLI evaluation configuration");
    let scenarios = load_scenarios(&settings).expect("load moat CLI evaluation scenarios");
    let mut versions = BTreeMap::new();
    for agent in &settings.agents {
        versions.insert(agent.id().to_string(), cli_version(*agent).await);
    }

    let mut trials = Vec::new();
    for scenario in &scenarios {
        for agent in &settings.agents {
            for repetition in 1..=settings.repetitions {
                println!(
                    "[{}][{}][{}/{}] running",
                    scenario.id,
                    agent.id(),
                    repetition,
                    settings.repetitions
                );
                let trial = run_trial(&settings, scenario, *agent, repetition).await;
                println!(
                    "[{}][{}][{}/{}] {} ({})",
                    scenario.id,
                    agent.id(),
                    repetition,
                    settings.repetitions,
                    if trial.passed { "PASS" } else { "FAIL" },
                    trial.terminal_outcome
                );
                trials.push(trial);
            }
        }
    }

    let summaries = summarize(&scenarios, &settings.agents, &trials, settings.min_passes);
    for summary in &summaries {
        println!(
            "[{}][{}] {}/{} passes; required {}",
            summary.scenario_id,
            summary.agent.id(),
            summary.passes,
            summary.trials,
            summary.required_passes
        );
    }
    let passed = summaries.iter().all(|summary| summary.passed);
    let report = EvalReport {
        commit: std::env::var("MOAT_CLI_EVAL_COMMIT").unwrap_or_else(|_| "unknown".to_string()),
        generated_at: Utc::now().to_rfc3339(),
        settings: EvalSettingsReport {
            repetitions: settings.repetitions,
            min_passes: settings.min_passes,
            scenarios: scenarios
                .iter()
                .map(|scenario| scenario.id.clone())
                .collect(),
            agents: settings
                .agents
                .iter()
                .map(|agent| agent.id().to_string())
                .collect(),
        },
        agent_cli_versions: versions,
        trials,
        summaries,
        passed,
    };
    write_report(&settings.report_path, &report)
        .await
        .expect("persist moat CLI evaluation report");
    println!("report: {}", settings.report_path.display());
    assert!(
        passed,
        "moat CLI evaluation did not meet its per-agent threshold; see {}",
        settings.report_path.display()
    );
}

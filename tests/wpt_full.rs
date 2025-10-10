use frontier::wpt::runner::{HarnessOutcome, WptManifest, WptRunner};
use std::collections::HashSet;
use tokio::runtime::Builder;

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
#[ignore]
fn wpt_full_timer_suite_summary() {
    let rt = runtime();
    rt.block_on(async {
        let runner = WptRunner::new("third_party/wpt").expect("runner");

        let curated_manifest =
            WptManifest::load_from_file("tests/wpt/manifest.txt").expect("load curated manifest");
        let expected_pass: HashSet<String> = curated_manifest
            .entries()
            .iter()
            .map(|entry| entry.to_string_lossy().into_owned())
            .collect();

        let full_manifest =
            WptManifest::load_from_file("tests/wpt/manifest_full.txt").expect("load full manifest");

        let mut evaluations = Vec::new();
        for entry in full_manifest.entries() {
            let entry_str = entry.to_string_lossy().into_owned();
            match runner.run_test(entry).await {
                Ok(run) => evaluations.push(TestEvaluation {
                    entry: entry_str,
                    run: Some(run),
                }),
                Err(err) => {
                    println!(
                        "⚠️  {} failed before harness completed:\n{}",
                        entry_str, err
                    );
                    evaluations.push(TestEvaluation {
                        entry: entry_str,
                        run: None,
                    });
                }
            }
        }

        report_results(evaluations, expected_pass);
    });
}

struct TestEvaluation {
    entry: String,
    run: Option<frontier::wpt::runner::WptRun>,
}

fn report_results(results: Vec<TestEvaluation>, expected_pass: HashSet<String>) {
    let total = results.len();
    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut timeout = 0usize;
    let mut precondition_failed = 0usize;

    let mut total_subtests = 0usize;
    let mut passed_subtests = 0usize;

    let mut regressions = Vec::new();
    let mut unexpected_passes = Vec::new();

    for evaluation in results {
        let entry = evaluation.entry;
        match evaluation.run {
            Some(outcome) => {
                let success = outcome.success();
                if success {
                    pass += 1;
                    if !expected_pass.contains(&entry) {
                        unexpected_passes.push(entry.clone());
                    }
                } else {
                    match outcome.harness_status.as_ref().map(|status| &status.status) {
                        Some(HarnessOutcome::Timeout) => timeout += 1,
                        Some(HarnessOutcome::PreconditionFailed) => precondition_failed += 1,
                        _ => fail += 1,
                    }

                    if expected_pass.contains(&entry) {
                        regressions.push(entry.clone());
                    }
                }

                for test in outcome.tests {
                    total_subtests += 1;
                    if test.status.is_pass() {
                        passed_subtests += 1;
                    }
                }
            }
            None => {
                fail += 1;
                if expected_pass.contains(&entry) {
                    regressions.push(entry.clone());
                }
            }
        }
    }

    println!("\n=== WPT Timer Suite Summary ===");
    println!("Total test files: {}", total);
    if total > 0 {
        println!(
            "  Passing: {} ({:.1}%)",
            pass,
            (pass as f64 / total as f64) * 100.0
        );
    } else {
        println!("  Passing: 0 (0.0%)");
    }
    println!("  Failing: {}", fail);
    println!("  Timeout: {}", timeout);
    println!("  Precondition failed: {}", precondition_failed);

    if total_subtests > 0 {
        println!(
            "Subtests passing: {} / {} ({:.1}%)",
            passed_subtests,
            total_subtests,
            (passed_subtests as f64 / total_subtests as f64) * 100.0
        );
    } else {
        println!("Subtests passing: 0 / 0 (0.0%)");
    }

    if !unexpected_passes.is_empty() {
        println!("\n🔥 Unexpected passes detected (update expectations):");
        for entry in &unexpected_passes {
            println!("  - {}", entry);
        }
    }

    if !regressions.is_empty() {
        println!("\n❌ Regressions detected (expected to pass):");
        for entry in &regressions {
            println!("  - {}", entry);
        }
    }

    if !regressions.is_empty() {
        panic!("{} WPT tests regressed", regressions.len());
    }

    if !unexpected_passes.is_empty() {
        println!(
            "\nℹ️  {} tests passed outside the curated manifest. Consider promoting them:",
            unexpected_passes.len()
        );
        for entry in unexpected_passes {
            println!("    {}", entry);
        }
    }
}

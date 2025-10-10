use frontier::wpt::runner::{HarnessOutcome, WptManifest, WptManifestResult, WptRunner, WptStatus};
use tokio::runtime::Builder;

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
fn inline_wpt_test_passes() {
    let rt = runtime();
    rt.block_on(async {
        let runner = WptRunner::new("third_party/wpt").expect("runner");
        let script = r#"
            test(() => {
                assert_true(true, 'boolean check');
            }, 'basic pass');
        "#;

        let run = runner
            .run_inline(script, "inline-basic.js")
            .await
            .expect("run WPT script");

        assert!(run.success(), "expected harness to report success");
        assert_eq!(run.tests.len(), 1);
        let test = &run.tests[0];
        assert_eq!(test.name, "basic pass");
        assert!(matches!(test.status, WptStatus::Pass));
        assert!(run
            .harness_status
            .as_ref()
            .map(|status| matches!(status.status, HarnessOutcome::Ok))
            .unwrap_or(true));
    });
}

#[test]
fn promise_test_executes_async_paths() {
    let rt = runtime();
    rt.block_on(async {
        let runner = WptRunner::new("third_party/wpt").expect("runner");
        let script = r#"
            promise_test(async () => {
                await new Promise((resolve) => {
                    setTimeout(resolve, 1);
                });
                assert_equals(document.body.nodeName, 'BODY');
            }, 'promise resolves after timeout');
        "#;

        let run = runner
            .run_inline(script, "inline-promise.js")
            .await
            .expect("run WPT promise script");

        assert!(run.success(), "expected asynchronous WPT run to succeed");
        assert_eq!(run.tests.len(), 1);
        assert!(matches!(run.tests[0].status, WptStatus::Pass));
    });
}

#[test]
fn failing_assertion_reports_failure() {
    let rt = runtime();
    rt.block_on(async {
        let runner = WptRunner::new("third_party/wpt").expect("runner");
        let script = r#"
            test(() => {
                assert_true(false, 'intentional failure');
            }, 'intentional failure');
        "#;

        let run = runner
            .run_inline(script, "inline-failure.js")
            .await
            .expect("run WPT failure script");

        assert!(!run.success(), "expected harness to report failure");
        assert_eq!(run.tests.len(), 1);
        let test = &run.tests[0];
        assert!(matches!(test.status, WptStatus::Fail));
        assert!(test
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("intentional failure"));
    });
}

#[test]
fn curated_manifest_executes_all_entries() {
    let rt = runtime();
    rt.block_on(async {
        let runner = WptRunner::new("third_party/wpt").expect("runner");
        let manifest =
            WptManifest::load_from_file("tests/wpt/manifest.txt").expect("load manifest");

        let results = runner.run_manifest(&manifest).await.expect("run manifest");

        assert!(!results.is_empty(), "manifest should contain entries");
        for WptManifestResult { entry, outcome } in results {
            assert!(
                outcome.success(),
                "manifest entry {} should succeed",
                entry.display()
            );
        }
    });
}

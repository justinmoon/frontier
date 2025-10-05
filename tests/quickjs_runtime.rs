use frontier::js::runtime::QuickJsEngine;

#[test]
fn quickjs_executes_inline_script() {
    let engine = QuickJsEngine::new().expect("engine");
    let result: i32 = engine
        .eval_with(
            "(() => { console.log('hello from test'); return 40 + 2; })()",
            "quickjs_runtime_test.js",
        )
        .expect("script result");
    assert_eq!(result, 42);
}

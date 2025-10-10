(() => {
  const mapTestStatus = (test) => {
    if (!test || typeof test.status !== 'number') {
      return 'UNKNOWN';
    }

    if ('PASS' in test && test.status === test.PASS) {
      return 'PASS';
    }
    if ('FAIL' in test && test.status === test.FAIL) {
      return 'FAIL';
    }
    if ('TIMEOUT' in test && test.status === test.TIMEOUT) {
      return 'TIMEOUT';
    }
    if ('NOTRUN' in test && test.status === test.NOTRUN) {
      return 'NOTRUN';
    }
    if ('PRECONDITION_FAILED' in test && test.status === test.PRECONDITION_FAILED) {
      return 'PRECONDITION_FAILED';
    }
    return String(test.status);
  };

  const mapHarnessStatus = (status) => {
    if (!status || typeof status.status !== 'number') {
      return null;
    }

    const result = {
      status: 'UNKNOWN',
      message: typeof status.message === 'undefined' ? null : status.message,
      stack: typeof status.stack === 'undefined' ? null : status.stack,
    };

    if ('OK' in status && status.status === status.OK) {
      result.status = 'OK';
      return result;
    }
    if ('ERROR' in status && status.status === status.ERROR) {
      result.status = 'ERROR';
      return result;
    }
    if ('TIMEOUT' in status && status.status === status.TIMEOUT) {
      result.status = 'TIMEOUT';
      return result;
    }
    if ('PRECONDITION_FAILED' in status && status.status === status.PRECONDITION_FAILED) {
      result.status = 'PRECONDITION_FAILED';
      return result;
    }

    result.status = String(status.status);
    return result;
  };

  const store = {
    done: false,
    tests: [],
    harnessStatus: null,
    asserts: [],
  };

  const toSerializableTest = (test) => ({
    name: typeof test.name === 'string' ? test.name : String(test.name),
    status: mapTestStatus(test),
    message: typeof test.message === 'undefined' ? null : test.message,
    stack: typeof test.stack === 'undefined' ? null : test.stack,
    index: typeof test.index === 'number' ? test.index : null,
  });

  const toSerializableAssert = (assert) => ({
    name: assert && 'assert_name' in assert ? assert.assert_name : null,
    status: assert && 'status' in assert ? String(assert.status) : null,
    stack: assert && 'stack' in assert ? (typeof assert.stack === 'undefined' ? null : assert.stack) : null,
    test: assert && assert.test ? toSerializableTest(assert.test) : null,
  });

  if (typeof add_result_callback === 'function') {
    add_result_callback((test) => {
      if (!store.done) {
        store.tests.push(toSerializableTest(test));
      }
    });
  }

  if (typeof add_completion_callback === 'function') {
    add_completion_callback((tests, harnessStatus, asserts) => {
      store.tests = Array.isArray(tests) ? tests.map(toSerializableTest) : [];
      store.harnessStatus = mapHarnessStatus(harnessStatus);
      store.asserts = Array.isArray(asserts) ? asserts.map(toSerializableAssert) : [];
      store.done = true;
    });
  }

  globalThis.__frontierWptIsDone = () => store.done;
  globalThis.__frontierWptSerialize = () => JSON.stringify(store);
})();

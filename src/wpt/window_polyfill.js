(function() {
  if (typeof window === 'undefined') {
    return;
  }

  if (typeof window.self === 'undefined') {
    window.self = window;
  }
  if (typeof window.parent === 'undefined') {
    window.parent = window;
  }
  if (typeof window.top === 'undefined') {
    window.top = window;
  }
  if (typeof window.opener === 'undefined') {
    window.opener = null;
  }

  var loadCallbacks = Array.isArray(window.__frontierLoadCallbacks)
    ? window.__frontierLoadCallbacks
    : [];

  try {
    Object.defineProperty(window, '__frontierLoadCallbacks', {
      value: loadCallbacks,
      writable: false,
      configurable: false,
    });
  } catch (err) {
    window.__frontierLoadCallbacks = loadCallbacks;
  }

  window.__frontierDispatchLoad = function() {
    for (var i = 0; i < loadCallbacks.length; i++) {
      try {
        loadCallbacks[i].call(window, { type: 'load', target: window });
      } catch (err) {
        // Ignore listener failures triggered from the polyfill.
      }
    }
  };

  if (typeof window.addEventListener !== 'function') {
    window.addEventListener = function(type, listener) {
      if (type === 'load' && typeof listener === 'function') {
        loadCallbacks.push(listener);
      }
    };
  }

  if (typeof window.removeEventListener !== 'function') {
    window.removeEventListener = function(type, listener) {
      if (type === 'load' && typeof listener === 'function') {
        for (var i = 0; i < loadCallbacks.length; i++) {
          if (loadCallbacks[i] === listener) {
            loadCallbacks.splice(i, 1);
            break;
          }
        }
      }
    };
  }

  if (typeof window.dispatchEvent !== 'function') {
    window.dispatchEvent = function(event) {
      if (event && event.type === 'load') {
        window.__frontierDispatchLoad();
      }
    };
  }

  if (typeof window.postMessage !== 'function') {
    window.postMessage = function() {};
  }

  if (typeof globalThis.self === 'undefined') {
    globalThis.self = globalThis;
  }

  if (typeof document !== 'undefined' && typeof document.getElementsByTagName !== 'function') {
    document.getElementsByTagName = function() {
      return [];
    };
  }
})();

# Blitz URL Bar Browser

A simple browser with a URL bar built on the blitz rendering engine.

## Quick Start

```bash
# Run the browser
just run
# or
cargo run

# Run with a specific URL
just run https://google.com
```

## Testing

```bash
# Fast offline test (no network)
just test-offline

# Online test (requires network)
just test-online

# All tests
just test-all
```

## How It Works

Type a URL in the address bar and press Enter or click "Go". That's it.

The URL bar is an HTML form that submits to blitz's navigation system. When you enter a URL:

1. Form submits with GET → creates `?url=` query parameter
2. Navigation handler extracts the actual URL
3. Fetches and renders the page
4. URL bar updates to show the final URL

## Development

```bash
just build        # Build
just build-release # Release build
just clean        # Clean build artifacts
```

## Files

- `src/main.rs` - URL bar wrapper and startup
- `src/readme_application.rs` - Navigation handling
- `tests/offline_test.rs` - Basic structure test
- `tests/online_test.rs` - Real webpage loading test

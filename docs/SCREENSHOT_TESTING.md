# Screenshot Testing

Visual testing for UI rendering bugs.

## Quick Start

```bash
# Screenshot an example (pass binary/example name)
./scripts/screenshot_app.sh frontier

# Screenshot main app
./scripts/screenshot_main_app.sh

# Run automated test
cargo test --test screenshot_test
```

Screenshots are saved to `/tmp/`:

- `/tmp/frontier_app.png`
- `/tmp/frontier_main_app.png`

## How It Works

The script:

1. Builds and launches the app
2. Finds window by PID using AppleScript
3. Captures window with `screencapture` (works even when app is in background)
4. Opens the image

## Manual Screenshot

```bash
cargo run --example minimal_dual_render &
sleep 3
screencapture -o /tmp/test.png
open /tmp/test.png
```

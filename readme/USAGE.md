# Blitz URL Bar Browser - Usage Guide

## Building and Running

### Build the browser
```bash
cargo build
```

### Run the browser
```bash
# Start with default URL (https://example.com)
cargo run

# Start with a specific URL
cargo run -- https://www.google.com
cargo run -- https://github.com
```

## Using the Browser

1. **Enter a URL**: Type any URL in the address bar
2. **Navigate**:
   - Click the "Go" button, OR
   - Press Enter while the URL input is focused
3. **Browse**: The page will load and display in the content area

### Supported URL formats:
- `https://example.com`
- `http://localhost:8080`
- `file:///path/to/file.html`
- `www.google.com` (will be converted to https://)

## Testing

### Run all unit tests (16 tests)
```bash
cargo test
```

### Run integration tests with real websites (3 tests)
These tests require network access and fetch real websites:
```bash
cargo test --test integration_test -- --ignored
```

## Test Suite Overview

### Unit Tests (15 tests)
Located in `tests/url_bar_navigation.rs`:

- **Structure Tests**: Verify URL bar, form, input, and submit button exist
- **Accessibility Tests**: Validate ARIA labels, roles, semantic HTML
- **Navigation Tests**: Check form submission setup and navigation flow
- **Input Validation**: Test URL type, required attributes, keyboard support
- **Integration**: Verify URL bar works with realistic page content

### Integration Tests (3 tests)
Located in `tests/integration_test.rs`:

1. **test_fetch_example_com**:
   - Fetches https://example.com
   - Verifies H1 contains "Example Domain"
   - Checks for paragraphs and links

2. **test_fetch_google_com**:
   - Fetches https://www.google.com
   - Verifies search form exists
   - Checks for input fields

3. **test_navigation_simulation**:
   - Simulates navigating between pages
   - Verifies different URLs load different content

## Accessibility Features

The URL bar is fully accessible:

- **ARIA labels**: All interactive elements have descriptive labels
- **Keyboard navigation**: Full keyboard support (Tab, Enter, arrow keys)
- **Screen reader support**: Proper semantic HTML and labels
- **Form validation**: URL type input with required attribute
- **Language support**: `lang="en"` attribute on HTML element

### Semantic HTML Structure:
```html
<nav role="navigation" aria-label="Browser navigation">
  <form role="search">
    <label for="url-input">Enter website URL</label>
    <input type="url" aria-label="Website URL address bar" />
    <input type="submit" aria-label="Navigate to URL" />
  </form>
</nav>
<main role="main" aria-label="Page content">
  <!-- Page content here -->
</main>
```

## Technical Implementation

### Form Submission
- Uses `<input type="submit">` instead of `<button>` for blitz compatibility
- Pressing Enter in URL input triggers form submission
- Click on "Go" button triggers form submission
- Form submission calls `doc.submit_form()` which triggers `NavigationProvider.navigate_to()`

### Navigation Flow
1. User enters URL and submits form
2. Blitz form system collects form data (the URL)
3. `NavigationProvider.navigate_to()` is called with the URL
4. Network provider fetches the new URL
5. Document is replaced with new content
6. URL bar is updated with the new URL

## Keyboard Shortcuts

While in the URL bar input:
- **Enter**: Submit form and navigate to URL
- **Cmd/Ctrl + A**: Select all text
- **Cmd/Ctrl + C**: Copy selected text
- **Cmd/Ctrl + V**: Paste text
- **Cmd/Ctrl + X**: Cut selected text
- **Arrow keys**: Move cursor
- **Home/End**: Jump to start/end

## Troubleshooting

### Navigation doesn't work
- Make sure you're using `<input type="submit">` not `<button type="submit">`
- Check that the input has `name="url"` attribute for form data collection
- Verify the form element wraps the input and submit button

### Tests failing
- Unit tests should always pass
- Integration tests require network access - they're marked `#[ignore]` by default
- Run integration tests explicitly with `--ignored` flag

### Build errors
- Make sure you have Rust 1.86.0 or later
- Check that blitz dependencies are correctly referenced
- Run `cargo clean` and rebuild if needed

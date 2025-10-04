#!/usr/bin/env python3
"""
Simple HTTP server for testing NNS resolution.
Serves a basic HTML page on port 8080.
"""

from http.server import HTTPServer, SimpleHTTPRequestHandler
import os

class TestHandler(SimpleHTTPRequestHandler):
    def do_GET(self):
        """Handle GET requests"""
        self.send_response(200)
        self.send_header('Content-type', 'text/html')
        self.end_headers()

        html = """<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>NNS Test Site</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
            max-width: 800px;
            margin: 50px auto;
            padding: 20px;
            background: #f6f8fa;
        }
        .container {
            background: white;
            border: 1px solid #d0d7de;
            border-radius: 6px;
            padding: 40px;
        }
        h1 {
            color: #24292f;
            margin-top: 0;
        }
        .success {
            background: #dafbe1;
            border: 1px solid #2da44e;
            border-radius: 6px;
            padding: 16px;
            margin: 20px 0;
        }
        .success strong {
            color: #1a7f37;
        }
        code {
            background: #f6f8fa;
            padding: 3px 6px;
            border-radius: 3px;
            font-family: ui-monospace, monospace;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>🎉 NNS Resolution Successful!</h1>
        <div class="success">
            <strong>Success!</strong> You've successfully accessed this site via the Nostr Name System (NNS).
        </div>
        <p>This page is being served from a simple HTTP server on <code>localhost:8080</code>.</p>
        <p>The browser resolved an NNS name to this IP:port combination by querying Nostr relays for kind 34256 events.</p>
        <h2>How it works:</h2>
        <ol>
            <li>You entered an NNS name in the browser's URL bar</li>
            <li>The browser queried Nostr relays for claims to that name</li>
            <li>You selected this server's claim (or it was auto-selected)</li>
            <li>The browser fetched this page via plain HTTP (no HTTPS needed for this demo)</li>
        </ol>
        <p><strong>Next steps:</strong> Try entering different NNS names or publish your own claims!</p>
    </div>
</body>
</html>"""

        self.wfile.write(html.encode())

def run_server(port=8080):
    server_address = ('', port)
    httpd = HTTPServer(server_address, TestHandler)
    print(f"🚀 Test HTTP server running on http://localhost:{port}")
    print("Press Ctrl+C to stop")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n\n👋 Server stopped")

if __name__ == '__main__':
    run_server()

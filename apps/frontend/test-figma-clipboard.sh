#!/bin/bash

# Figma Clipboard Test Helper
# This script opens the Figma clipboard test page in your default browser

set -e

cd "$(dirname "$0")"

echo "🎨 Figma Clipboard Test Helper"
echo "================================"
echo ""

# Check if the test file exists
if [ ! -f "test-figma-paste.html" ]; then
    echo "❌ Error: test-figma-paste.html not found"
    exit 1
fi

# Get the absolute path
TEST_FILE="$(pwd)/test-figma-paste.html"
echo "📄 Test file: $TEST_FILE"
echo ""

# Try to find the backend fixtures path
FIXTURE_PATH="$(pwd)/../backend/src/services/__tests__/fixtures/test.html"
if [ ! -f "$FIXTURE_PATH" ]; then
    echo "⚠️  Warning: Test fixture not found at $FIXTURE_PATH"
    echo "   The page will try to load it via fetch, which might fail due to CORS"
    echo ""
fi

echo "🌐 Opening test page in browser..."
echo ""
echo "Instructions:"
echo "1. Click 'Copy Figma Test Data' button"
echo "2. Go to your app (http://localhost:3000)"
echo "3. Paste (Cmd+V) in any textarea with the Figma paste handler"
echo ""
echo "Or paste directly in the test page textarea to verify the clipboard data"
echo ""

# Open the file in the default browser
if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS
    open "$TEST_FILE"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    # Linux
    xdg-open "$TEST_FILE"
elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" ]]; then
    # Windows
    start "$TEST_FILE"
else
    echo "❌ Unsupported OS: $OSTYPE"
    echo "Please open the file manually: file://$TEST_FILE"
    exit 1
fi

echo "✅ Test page opened!"
echo ""
echo "💡 Tip: If the fetch fails due to CORS, you can:"
echo "   1. Start a simple HTTP server: python3 -m http.server 8080"
echo "   2. Navigate to: http://localhost:8080/test-figma-paste.html"

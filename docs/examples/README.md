# Figma Import Examples

This directory contains examples of Figma import prompts showing the two different import formats available in the application.

## Available Examples

### [Clipboard Import Example](./figma-clipboard-import-real-example.md)

A **real, production-quality example** showing the exact prompt format generated when importing a Figma design via clipboard paste.

**Details:**
- **Source**: Actual test fixture (`apps/backend/src/services/__tests__/fixtures/figma-clipboard.html`)
- **Component**: Customer details page with risk indicators, badges, and data table
- **Nodes**: 289 nodes across 12 levels of hierarchy
- **Format**: fig-kiwie binary clipboard format
- **Has Image**: No (clipboard imports don't include images)

**What you'll see:**
- Complete hierarchical node structure
- Tailwind-friendly CSS property mappings
- Auto-layout specifications (horizontal/vertical, gaps, padding)
- Typography details (fonts, sizes, weights, alignment)
- Color values in hex format
- Design token usage indicators (🎨)
- Component sizing modes (hug vs fixed)

This example shows exactly what AI receives when you:
1. Select a component in Figma
2. Copy it (Cmd+C / Ctrl+C)
3. Paste into the chat input
4. The system automatically detects and parses the clipboard data

### [REST API Import Example](./figma-rest-api-import-example.md)

A **mock example** showing the REST API import format for comparison purposes.

**Details:**
- **Source**: Mock REST API Node (created for comparison)
- **Component**: Customer details frame (simplified)
- **Format**: Figma REST API format
- **Has Image**: Yes (when `FIGMA_IMAGES_ENABLED=true`)

**What you'll see:**
- Official Figma Node structure
- Simpler, cleaner property names
- Standard Figma types (layoutMode, fills, strokes)
- Absolute bounding boxes for positioning
- Typography via style objects
- Color values as RGBA decimals

This example shows what AI receives when you:
1. Paste a Figma URL in the import dialog
2. Backend fetches node data from Figma REST API
3. Optional image download (if enabled)
4. Prompt generated from REST API format

## Format Comparison

The two import formats have different strengths and are suited for different use cases:

### Clipboard Format (fig-kiwie)

**Best for:** Quick iterations, no setup required, detailed CSS-like specifications

**Pros**:
- ✅ Instant parsing (no API calls)
- ✅ No API token required
- ✅ Rich layout metadata with Tailwind-friendly property names
- ✅ Design token indicators (🎨)
- ✅ Sizing modes (hug/fixed/constraints)
- ✅ More detailed CSS-like properties (gap, padding syntax matches CSS)
- ✅ Works offline

**Cons**:
- ❌ No visual image included
- ❌ Only text-based specifications
- ❌ Must manually copy from Figma each time

**Property Examples:**
```
- Layout: Vertical, gap-16px, pb-88px
- Fill: #FFFFFF
- Border: 1px border, #E4E4E7
```

### REST API Format

**Best for:** Persistent imports with images, official Figma types, visual reference

**Pros**:
- ✅ Includes rendered image preview (when `FIGMA_IMAGES_ENABLED=true`)
- ✅ Official Figma API types
- ✅ Visual reference for AI
- ✅ Simpler, cleaner structure
- ✅ Can be refreshed without re-copying

**Cons**:
- ❌ Requires `FIGMA_ACCESS_TOKEN` environment variable
- ❌ Slower (API roundtrips to Figma servers)
- ❌ Less detailed layout metadata
- ❌ No Tailwind-friendly property names
- ❌ No design token indicators
- ❌ Requires internet connection

**Property Examples:**
```
layoutMode: "VERTICAL"
itemSpacing: 16
paddingBottom: 88
fills: [{ type: "SOLID", color: { r: 1, g: 1, b: 1 } }]
```

### Key Differences Summary

| Feature | Clipboard | REST API |
|---------|-----------|----------|
| **Setup** | None | Requires API token |
| **Speed** | Instant | 2-3 second API calls |
| **Image** | ❌ No | ✅ Yes (optional) |
| **Property Names** | CSS-like (`gap-16px`) | Figma official (`itemSpacing: 16`) |
| **Design Tokens** | ✅ Indicated with 🎨 | ❌ Not marked |
| **Offline** | ✅ Works | ❌ Needs internet |
| **Refresh** | Manual re-copy | Can re-fetch |

## Generating New Examples

To regenerate both examples from fixture data:

```bash
cd apps/backend
bun run src/scripts/generate-figma-examples.ts
```

This will:
1. **Clipboard Format:**
   - Load the real Figma clipboard HTML from test fixtures
   - Parse the binary fig-kiwie format (289 nodes)
   - Build the complete node tree hierarchy
   - Generate the formatted prompt text using `generateClipboardContextText()`
   - Save to `docs/examples/figma-clipboard-import-real-example.md`

2. **REST API Format:**
   - Create a mock REST API Node structure
   - Set `FIGMA_IMPORT_FORMAT=rest-api` environment variable
   - Generate the formatted prompt text using `generateFigmaContextText()`
   - Save to `docs/examples/figma-rest-api-import-example.md`

The script demonstrates how both transformers work and shows the structural differences between the two formats.

## Node Type Distribution

The real example contains a diverse set of Figma node types:

- **VARIABLE**: 89 nodes (design tokens/variables)
- **FRAME**: 83 nodes (containers with auto-layout)
- **TEXT**: 80 nodes (text content)
- **INSTANCE**: 16 nodes (component instances)
- **SYMBOL**: 10 nodes (component definitions)
- **VARIABLE_SET**: 5 nodes (design token collections)
- **CANVAS**: 2 nodes (pages)
- **VECTOR**: 2 nodes (icons/graphics)
- **DOCUMENT**: 1 node (root)
- **LINE**: 1 node (divider)

This diversity ensures the example demonstrates how the parser handles all common Figma node types.

## Use Cases

These examples are helpful for:

1. **Understanding prompt format**: See exactly what AI receives
2. **Testing prompt generators**: Verify your transformers produce similar output
3. **Documentation**: Show developers what to expect
4. **API design**: Understand the data structure for frontend/backend integration
5. **Training**: Help new team members understand the Figma import feature

## Related Documentation

### Feature Documentation
- [Figma Import Feature](../patterns/figma-import.md) - User guide and patterns

### Implementation Files
- [Figma Service](../../apps/backend/src/services/figma.service.ts) - Main service handling both import methods
- [Clipboard Parser](../../apps/backend/src/services/figma-clipboard-parser.ts) - Binary fig-kiwie format parser
- [Clipboard Transformers](../../apps/backend/src/utils/figma-clipboard-transformers.ts) - Clipboard format prompt generator
- [REST API Transformers](../../apps/backend/src/utils/figma-transformers.ts) - REST API format prompt generator
- [Example Generator Script](../../apps/backend/src/scripts/generate-figma-examples.ts) - Script to regenerate examples

### Tests
- [Clipboard Parser Integration Tests](../../apps/backend/src/services/__tests__/figma-clipboard-parser.integration.test.ts) - Test coverage for clipboard parsing

# @moatless/tagger

Automatic JSX/TSX component tagging for debugging in both Vite and Webpack builds.

## Overview

The component tagger automatically adds data attributes to JSX/TSX components during development to help with:
- Component debugging and inspection
- Visual component identification in DevTools
- Automated testing and E2E workflows
- Component usage analytics

These attributes are **only added in development mode** and can be configured to exclude production builds.

## Installation

```bash
npm install @moatless/tagger
# or
yarn add @moatless/tagger
# or
pnpm add @moatless/tagger
# or
bun add @moatless/tagger
```

## Usage

### Vite

Add the plugin to your `vite.config.ts`:

```typescript
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import componentTagger from '@moatless/tagger/vite';

export default defineConfig({
  plugins: [
    react(),
    componentTagger(), // Only runs in dev mode by default
  ],
});
```

#### Vite Options

```typescript
componentTagger({
  // Only apply in development mode (default: true)
  devOnly: true,

  // Include/exclude patterns (default: /\.(jsx|tsx)$/)
  include: /\.(jsx|tsx)$/,
  exclude: /node_modules/,

  // Custom parser plugins
  parserPlugins: ['jsx', 'typescript'],

  // Custom filter function
  shouldTag: (elementName: string) => {
    return !elementName.startsWith('Internal');
  },
});
```

### Webpack / Next.js

#### Standard Webpack

Add the loader to your webpack config:

```javascript
module.exports = {
  module: {
    rules: [
      {
        test: /\.(jsx|tsx)$/,
        exclude: /node_modules/,
        use: [
          {
            loader: '@moatless/tagger/webpack',
          },
        ],
      },
    ],
  },
};
```

#### Next.js

Add the loader in your `next.config.js` or `next.config.ts`:

```javascript
module.exports = {
  webpack: (config, { dev }) => {
    if (dev) {
      config.module.rules.push({
        test: /\.(jsx|tsx)$/,
        exclude: /node_modules/,
        use: [
          {
            loader: require.resolve('@moatless/tagger/webpack'),
          },
        ],
      });
    }
    return config;
  },
};
```

## What It Does

For each JSX component in your codebase, the tagger adds these data attributes:

```html
<Button
  data-component-id="src/components/ui/button.tsx:15:3"
  data-component-name="Button"
>
  Click me
</Button>
```

### Attributes Reference

| Attribute | Description | Example |
|-----------|-------------|---------|
| `data-component-id` | Unique identifier (path:line:column) | `src/app/page.tsx:42:5` |
| `data-component-name` | Component/element name | `Button`, `div`, `MyComponent` |

## Use Cases

### 1. Component Inspector Tool

Build a browser extension or dev tool to identify components:

```javascript
document.addEventListener('click', (e) => {
  const id = e.target.getAttribute('data-component-id');
  const name = e.target.getAttribute('data-component-name');
  console.log(`Clicked ${name} at ${id}`);
  // Opens file in your editor at the exact line
});
```

### 2. Visual Debugging

Highlight specific components in the browser:

```javascript
// Highlight all Button components
document.querySelectorAll('[data-component-name="Button"]')
  .forEach(el => el.style.outline = '2px solid red');

// Find components from a specific file
document.querySelectorAll('[data-component-path*="src/app/alerts"]')
  .forEach(el => el.style.backgroundColor = 'rgba(255,0,0,0.1)');
```

### 3. Component Usage Analytics

Track which components are rendered:

```javascript
const components = Array.from(
  document.querySelectorAll('[data-component-id]')
).map(el => ({
  name: el.getAttribute('data-component-name'),
  location: el.getAttribute('data-component-id'),
  file: el.getAttribute('data-component-file')
}));

console.log('Rendered components:', components);
```

### 4. Automated Testing

Use component attributes in tests:

```javascript
// Cypress or Playwright
cy.get('[data-component-name="LoginButton"]').click();

// Find component by file
cy.get('[data-component-file="login-form.tsx"]').should('be.visible');
```

## Configuration

### What Gets Tagged

- ✅ All JSX/TSX components in your source directory
- ✅ Custom React components (`<Button>`, `<MyComponent>`)
- ✅ HTML elements (`<div>`, `<span>`, `<button>`)

### What Gets Excluded

- ❌ Files in `node_modules`
- ❌ React Fragments (`<Fragment>`, `<React.Fragment>`)
- ❌ Three.js/Fiber elements (mesh, camera, light, geometry, material, etc.)
- ❌ `@react-three/drei` components (OrbitControls, Environment, etc.)
- ❌ Production builds (when `devOnly: true`)

### Custom Filtering

You can provide a custom `shouldTag` function to control which elements get tagged:

```typescript
componentTagger({
  shouldTag: (elementName: string) => {
    // Skip all internal components
    if (elementName.startsWith('Internal')) {
      return false;
    }
    // Skip specific component
    if (elementName === 'NoTagComponent') {
      return false;
    }
    return true;
  },
});
```

## Advanced Usage

### Programmatic API

You can also use the core transformation function directly:

```typescript
import { transformComponentTags } from '@moatless/tagger';

const result = transformComponentTags(
  sourceCode,
  filePath,
  projectRoot,
  {
    parserPlugins: ['jsx', 'typescript'],
    shouldTag: (elementName) => !elementName.startsWith('_'),
  }
);

console.log(result.code); // Transformed code
console.log(result.map);  // Source map
```

### TypeScript Support

Full TypeScript support with exported types:

```typescript
import type {
  TaggerOptions,
  VitePluginOptions,
  TagResult,
} from '@moatless/tagger';
```

## Performance

- **Build Time**: Minimal impact (~100-200ms for typical project)
- **Runtime**: No runtime overhead (attributes are static HTML)
- **File Size**: ~20-40 bytes per component in development
- **Production**: Zero impact when `devOnly: true` (default)

## Browser DevTools Usage

### Inspect Component Source

1. Right-click any element in the browser
2. Select "Inspect Element"
3. Look for `data-component-id` in the attributes panel
4. Use the path to navigate to the source file

Example:
```
data-component-id="src/components/ui/button.tsx:15:3"
                   └─ file path ──────┘ └line┘└col┘
```

### Console Debugging

```javascript
// Find all Button components
$$('[data-component-name="Button"]')

// Get component location
$0.getAttribute('data-component-id') // "src/components/ui/button.tsx:15:3"

// Count component usage
$$('[data-component-name="Button"]').length
```

## Troubleshooting

### Attributes Not Showing Up

1. **Check dev mode**: Attributes only appear in development with `devOnly: true`
2. **Clear cache**: Delete build cache (`.next`, `node_modules/.vite`, etc.)
3. **Check browser**: Use DevTools Inspector, not "View Page Source"

### Build Errors

1. **Missing dependencies**: Run `npm install` or your package manager
2. **TypeScript errors**: Run `npm run type:check` to verify types
3. **Check file extensions**: Only `.jsx` and `.tsx` files are processed by default

## Credits

This package is based on [lovable-tagger](https://www.npmjs.com/package/lovable-tagger), extended to support both Vite and Webpack build systems with a modernized TypeScript implementation.

## License

MIT

# Finding and Debugging React Performance Issues

## Tools and Techniques

### 1. React DevTools Profiler
The most powerful tool for finding performance issues:

```bash
# Install React DevTools browser extension
# Chrome: https://chrome.google.com/webstore/detail/react-developer-tools/
# Firefox: https://addons.mozilla.org/en-US/firefox/addon/react-devtools/
```

**How to use:**
1. Open React DevTools → Profiler tab
2. Click "Start profiling" 
3. Interact with your app
4. Stop profiling and analyze:
   - Look for components that render frequently (bright yellow/orange)
   - Check "Ranked" view to see slowest components
   - Enable "Record why each component rendered"

**What to look for:**
- Components rendering on every parent render
- Cascading updates (one update triggering many others)
- Components with high render times

### 2. Console Logging for Render Detection

Add temporary logging to detect unnecessary renders:

```typescript
// Add to any component you suspect
console.log(`${ComponentName} rendered`, { props, state });

// Or use a custom hook
function useWhyDidYouUpdate(name: string, props: Record<string, any>) {
  const previousProps = useRef<Record<string, any>>();
  
  useEffect(() => {
    if (previousProps.current) {
      const allKeys = Object.keys({ ...previousProps.current, ...props });
      const changedProps: Record<string, any> = {};
      
      allKeys.forEach(key => {
        if (previousProps.current![key] !== props[key]) {
          changedProps[key] = {
            from: previousProps.current![key],
            to: props[key]
          };
        }
      });
      
      if (Object.keys(changedProps).length) {
        console.log('[why-did-you-update]', name, changedProps);
      }
    }
    
    previousProps.current = props;
  });
}
```

### 3. ESLint Rules for React Hooks

Enable exhaustive deps checking:

```json
// .eslintrc.json
{
  "extends": ["react-app"],
  "rules": {
    "react-hooks/rules-of-hooks": "error",
    "react-hooks/exhaustive-deps": "warn"
  }
}
```

### 4. Custom ESLint Rules

Create rules to catch common patterns:

```javascript
// eslint-rules/no-effect-for-derived-state.js
module.exports = {
  create(context) {
    return {
      CallExpression(node) {
        if (node.callee.name === 'useEffect') {
          const [effectFn] = node.arguments;
          if (effectFn && effectFn.body) {
            // Check if effect only calls setState
            const hasOnlySetState = /* logic to detect setState only */;
            if (hasOnlySetState) {
              context.report({
                node,
                message: 'Consider deriving state instead of using effect'
              });
            }
          }
        }
      }
    };
  }
};
```

### 5. TypeScript Strict Mode

Enable strict mode to catch more issues:

```json
// tsconfig.json
{
  "compilerOptions": {
    "strict": true,
    "noImplicitAny": true,
    "strictNullChecks": true,
    "noUnusedParameters": true
  }
}
```

### 6. React Compiler (Experimental)

The new React Compiler can automatically optimize many patterns:

```bash
# When it becomes stable
npm install react-compiler
```

### 7. Performance Testing

Write tests that catch performance regressions:

```typescript
import { render } from '@testing-library/react';
import { measureRenders } from './test-utils';

test('TaskProvider should not cause excessive renders', () => {
  const renderCount = measureRenders(() => {
    const { rerender } = render(<TaskProvider taskId="1">...</TaskProvider>);
    rerender(<TaskProvider taskId="1">...</TaskProvider>);
  });
  
  expect(renderCount).toBeLessThan(3);
});
```

## Common Patterns to Look For

### 1. Effects That Set State Based on Props
```typescript
// 🚨 Bad
useEffect(() => {
  setDerivedValue(calculateFromProps(props));
}, [props]);

// ✅ Good
const derivedValue = calculateFromProps(props);
```

### 2. Unstable Dependencies
```typescript
// 🚨 Bad - creates new object every render
const config = { foo: 'bar' };
useEffect(() => {}, [config]);

// ✅ Good
const config = useMemo(() => ({ foo: 'bar' }), []);
```

### 3. Context Value Changes
```typescript
// 🚨 Bad - new object every render
<Context.Provider value={{ state, setState }}>

// ✅ Good
const value = useMemo(() => ({ state, setState }), [state]);
<Context.Provider value={value}>
```

### 4. Inline Functions in Dependencies
```typescript
// 🚨 Bad
useEffect(() => {}, [() => doSomething()]);

// ✅ Good
const handler = useCallback(() => doSomething(), []);
useEffect(() => {}, [handler]);
```

## Automated Detection Scripts

### Find Potential Issues with Grep
```bash
# Find effects that might be deriving state
rg "useEffect.*setState" --type ts --type tsx

# Find inline objects in dependency arrays
rg "useEffect.*\[\{" --type ts --type tsx

# Find effects with props in dependencies
rg "useEffect.*\[.*props" --type ts --type tsx

# Find context providers with inline values
rg "<\w+Context\.Provider\s+value=\{\{" --type ts --type tsx
```

### Custom Script to Analyze Components
```typescript
// scripts/analyze-renders.ts
import { Project } from 'ts-morph';

const project = new Project();
project.addSourceFilesAtPaths('src/**/*.tsx');

project.getSourceFiles().forEach(file => {
  // Find all useEffect calls
  const effectCalls = file.getDescendantsOfKind(
    ts.SyntaxKind.CallExpression
  ).filter(call => call.getExpression().getText() === 'useEffect');
  
  effectCalls.forEach(effect => {
    // Analyze effect body for setState patterns
    const body = effect.getArguments()[0];
    if (containsOnlySetState(body)) {
      console.log(`Potential derived state in ${file.getFilePath()}`);
    }
  });
});
```

## Prevention Strategies

1. **Code Review Checklist**
   - Check all `useEffect` calls - are they necessary?
   - Verify context value stability
   - Look for derived state opportunities
   - Check dependency arrays for stability

2. **Team Guidelines**
   - Document common patterns in your codebase
   - Create reusable hooks for common patterns
   - Regular performance audits

3. **CI/CD Integration**
   - Run performance tests in CI
   - Use bundle size tracking
   - Set up alerts for performance regressions

## Quick Audit Commands

Run these in your project to find potential issues:

```bash
# Find all useEffect usage
echo "=== useEffect usage ==="
rg "useEffect" --type ts --type tsx -c | sort -t: -k2 -nr | head -20

# Find setState in effects
echo "=== setState in effects ==="
rg "useEffect\(.*\n.*setState" -U --type ts --type tsx

# Find potential context issues
echo "=== Inline context values ==="
rg "Provider.*value=\{\{" --type ts --type tsx

# Find components with many hooks (complexity indicator)
echo "=== Hook-heavy components ==="
rg "^  (use[A-Z]\w+)" --type ts --type tsx -c | sort -t: -k2 -nr | head -10
```
# Action Plan: Eliminating Hardcoded Colors

## 🎯 Goal
Replace all hardcoded color values with semantic tokens to ensure consistent theming and automatic light/dark mode switching.

## 🔍 Phase 1: Discovery & Audit

### 1.1 Run Automated Scans
```bash
# Create a comprehensive audit script
#!/bin/bash

echo "=== HARDCODED COLOR AUDIT ==="
echo ""

echo "1. Gray scale colors (gray, slate, zinc, stone, neutral):"
grep -r "text-\(gray\|slate\|zinc\|stone\|neutral\)-[0-9]" --include="*.tsx" --include="*.jsx" apps/frontend/src | wc -l
grep -r "bg-\(gray\|slate\|zinc\|stone\|neutral\)-[0-9]" --include="*.tsx" --include="*.jsx" apps/frontend/src | wc -l
grep -r "border-\(gray\|slate\|zinc\|stone\|neutral\)-[0-9]" --include="*.tsx" --include="*.jsx" apps/frontend/src | wc -l

echo ""
echo "2. Black/White hardcoded:"
grep -r "\(bg-white\|bg-black\|text-white\|text-black\)" --include="*.tsx" --include="*.jsx" apps/frontend/src | wc -l

echo ""
echo "3. Dark mode conditionals:"
grep -r "dark:" --include="*.tsx" --include="*.jsx" apps/frontend/src | grep -v "dark:.*-foreground\|dark:.*-background" | wc -l

echo ""
echo "4. Other color scales:"
grep -r "\(text\|bg\|border\)-\(red\|blue\|green\|yellow\|orange\|purple\|pink\)-[0-9]" --include="*.tsx" --include="*.jsx" apps/frontend/src | wc -l
```

### 1.2 Generate Detailed Report
```bash
# Generate a file-by-file report
find apps/frontend/src -name "*.tsx" -o -name "*.jsx" | while read file; do
  count=$(grep -E "(text|bg|border)-(gray|slate|zinc|stone|neutral|white|black|red|blue|green)-[0-9]|dark:" "$file" | wc -l)
  if [ $count -gt 0 ]; then
    echo "$file: $count hardcoded colors"
  fi
done | sort -t: -k2 -nr > hardcoded-colors-report.txt
```

### 1.3 Categorize Issues
Create a spreadsheet or tracking document with:
- **Critical** - Components with >10 hardcoded colors
- **High** - Components with 5-10 hardcoded colors  
- **Medium** - Components with 2-4 hardcoded colors
- **Low** - Components with 1 hardcoded color

## 🔄 Phase 2: Migration Strategy

### 2.1 Color Mapping Reference
Create a mapping guide for common replacements:

| Hardcoded Color | Semantic Token | Usage |
|-----------------|----------------|-------|
| `text-gray-900`, `text-black` | `text-foreground` | Primary text |
| `text-gray-600`, `text-gray-500` | `text-muted-foreground` | Secondary text |
| `bg-white` | `bg-background` or `bg-card` | Backgrounds |
| `bg-gray-50`, `bg-gray-100` | `bg-muted` | Subtle backgrounds |
| `border-gray-200`, `border-gray-300` | `border-border` | All borders |
| `text-white` on dark bg | `text-primary-foreground` | Text on primary |
| `bg-gray-900` (dark mode) | Use semantic token | Dark backgrounds |
| `hover:bg-gray-100` | `hover:bg-accent` | Hover states |
| `text-red-500`, `text-red-600` | `text-destructive` | Error text |
| `bg-red-500` | `bg-destructive` | Error backgrounds |

### 2.2 Component Priority List
Start with the most visible/used components:
1. **Navigation components** (Sidebar, Header)
2. **Form components** (Input, Button, Select)
3. **Layout components** (Card, Panel)
4. **Feature components** (Task lists, Chat)
5. **Utility components** (Tooltips, Modals)

## 🛠️ Phase 3: Implementation

### 3.1 Migration Template
For each component:

```tsx
// BEFORE: Hardcoded colors
<div className="bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 border-gray-200 dark:border-gray-800">
  <p className="text-gray-500 dark:text-gray-400">Secondary text</p>
  <button className="hover:bg-gray-100 dark:hover:bg-gray-800">Click</button>
</div>

// AFTER: Semantic tokens
<div className="bg-card text-card-foreground border-border">
  <p className="text-muted-foreground">Secondary text</p>
  <button className="hover:bg-accent hover:text-accent-foreground">Click</button>
</div>
```

### 3.2 Step-by-Step Process
1. **Open component file**
2. **Search for color patterns**: `gray-`, `slate-`, `white`, `black`, `dark:`
3. **Replace with semantic tokens** using the mapping guide
4. **Test in both themes**: Toggle between light and dark mode
5. **Verify no regressions**: Check hover states, focus states, disabled states
6. **Commit with clear message**: `fix: migrate [Component] to semantic color tokens`

### 3.3 Edge Cases

#### Custom Brand Colors
```tsx
// If you need specific brand colors not in the palette
<div className="bg-[rgb(252,251,248)]"> // OK for specific brand requirements

// But prefer adding to globals.css if used multiple times:
:root {
  --brand-warm: rgb(252 251 248);
}
// Then use: bg-[var(--brand-warm)]
```

#### Opacity Modifiers
```tsx
// BEFORE
<div className="bg-gray-900/50">

// AFTER - Use semantic token with opacity
<div className="bg-foreground/50">
```

## ✅ Phase 4: Validation

### 4.1 Automated Checks
Add to CI/CD pipeline:

```json
// package.json
{
  "scripts": {
    "lint:colors": "node scripts/check-hardcoded-colors.js",
    "pre-commit": "pnpm lint:colors"
  }
}
```

### 4.2 Create Linting Script
```javascript
// scripts/check-hardcoded-colors.js
const fs = require('fs');
const path = require('path');
const glob = require('glob');

const FORBIDDEN_PATTERNS = [
  /\b(text|bg|border)-(gray|slate|zinc|stone|neutral)-\d{1,3}\b/,
  /\b(text|bg|border)-(black|white)\b/,
  /\bdark:(text|bg|border)-/,
];

const ALLOWED_EXCEPTIONS = [
  'storybook', // Storybook files may need specific colors
  'test',      // Test files
  '.gen.ts',   // Generated files
];

function checkFile(filepath) {
  if (ALLOWED_EXCEPTIONS.some(exception => filepath.includes(exception))) {
    return [];
  }
  
  const content = fs.readFileSync(filepath, 'utf8');
  const lines = content.split('\n');
  const issues = [];
  
  lines.forEach((line, index) => {
    FORBIDDEN_PATTERNS.forEach(pattern => {
      if (pattern.test(line)) {
        issues.push({
          file: filepath,
          line: index + 1,
          content: line.trim(),
          pattern: pattern.toString()
        });
      }
    });
  });
  
  return issues;
}

// Run the check
const files = glob.sync('apps/frontend/src/**/*.{tsx,jsx}');
const allIssues = files.flatMap(checkFile);

if (allIssues.length > 0) {
  console.error('❌ Found hardcoded colors:');
  allIssues.forEach(issue => {
    console.error(`  ${issue.file}:${issue.line} - ${issue.content}`);
  });
  process.exit(1);
} else {
  console.log('✅ No hardcoded colors found!');
}
```

### 4.3 Code Review Checklist
- [ ] No `gray-XXX`, `slate-XXX`, etc. color classes
- [ ] No `dark:` prefixes for colors
- [ ] No `bg-white`, `bg-black`, `text-white`, `text-black`
- [ ] All colors use semantic tokens
- [ ] Component works in both light and dark themes
- [ ] No theme-specific bugs

## 📊 Phase 5: Monitoring & Maintenance

### 5.1 Regular Audits
- Run color audit script weekly
- Track progress in a dashboard
- Set targets (e.g., reduce by 20% each sprint)

### 5.2 Developer Education
- Update onboarding docs
- Create a "Color Usage Guide"
- Show before/after examples
- Regular team reminders

### 5.3 Tooling Improvements
- VS Code snippets for common patterns
- ESLint rule for hardcoded colors
- Automated PR comments for violations

## 🎯 Success Metrics

- **0 hardcoded colors** in new components
- **< 5% hardcoded colors** in legacy code after 3 months
- **100% theme switching** works without `dark:` classes
- **Reduced CSS bundle size** (fewer color variations)

## 📅 Timeline Estimate

- **Week 1**: Discovery & Audit
- **Week 2-3**: High-priority components
- **Week 4-5**: Medium-priority components  
- **Week 6**: Low-priority & edge cases
- **Week 7**: Validation & documentation
- **Week 8**: Tooling & process improvements

## 🚨 Common Pitfalls to Avoid

1. **Don't rush** - Test each change in both themes
2. **Don't over-abstract** - Use existing semantic tokens first
3. **Don't break designs** - Some opacity/shades might need custom handling
4. **Don't forget states** - Check hover, focus, active, disabled
5. **Don't ignore warnings** - If a component looks wrong, investigate

## 📚 Resources

- [Styling Guide](/docs/styling-guide.md) - Complete styling documentation
- [Semantic Token Reference](#) - Quick lookup table
- [Migration Examples](#) - Before/after code samples
- [Theme Testing Guide](#) - How to properly test themes

---

**Remember**: The goal isn't perfection on day one, but consistent progress toward a maintainable, themeable codebase. Each hardcoded color replaced is a step toward better UX and easier maintenance.
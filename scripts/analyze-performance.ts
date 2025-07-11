#!/usr/bin/env tsx

import { readFileSync, readdirSync, statSync } from 'fs';
import { join } from 'path';

interface Issue {
  file: string;
  line: number;
  type: string;
  description: string;
  code: string;
}

const issues: Issue[] = [];

function findTsxFiles(dir: string): string[] {
  const files: string[] = [];
  
  try {
    const items = readdirSync(dir);
    
    for (const item of items) {
      const fullPath = join(dir, item);
      const stat = statSync(fullPath);
      
      if (stat.isDirectory() && !item.includes('node_modules') && !item.includes('__tests__')) {
        files.push(...findTsxFiles(fullPath));
      } else if (item.endsWith('.tsx') || item.endsWith('.ts')) {
        files.push(fullPath);
      }
    }
  } catch (e) {
    // Skip directories we can't read
  }
  
  return files;
}

function analyzeFile(filePath: string) {
  const content = readFileSync(filePath, 'utf-8');
  const lines = content.split('\n');
  
  // Pattern 1: useEffect that only sets state
  const effectPattern = /useEffect\s*\(/;
  const setStatePattern = /set[A-Z]\w*\s*\(/;
  
  lines.forEach((line, index) => {
    if (effectPattern.test(line)) {
      // Look ahead for setState calls
      let foundSetState = false;
      let hasOtherLogic = false;
      
      for (let i = 1; i < 10 && index + i < lines.length; i++) {
        const nextLine = lines[index + i];
        if (setStatePattern.test(nextLine)) {
          foundSetState = true;
        }
        if (nextLine.includes('if (') || nextLine.includes('for (') || nextLine.includes('fetch')) {
          hasOtherLogic = true;
        }
        if (nextLine.includes('}, [')) break;
      }
      
      if (foundSetState && !hasOtherLogic) {
        issues.push({
          file: filePath,
          line: index + 1,
          type: 'DERIVED_STATE',
          description: 'useEffect that only sets state - consider deriving',
          code: line.trim()
        });
      }
    }
    
    // Pattern 2: Context.Provider with inline value
    if (line.includes('.Provider') && line.includes('value={{')) {
      issues.push({
        file: filePath,
        line: index + 1,
        type: 'INLINE_CONTEXT',
        description: 'Context Provider with inline object value',
        code: line.trim()
      });
    }
    
    // Pattern 3: Dependencies with inline objects/functions
    const depsPattern = /\[[^\]]*[{(]/;
    if ((line.includes('useEffect') || line.includes('useMemo') || line.includes('useCallback')) && depsPattern.test(line)) {
      issues.push({
        file: filePath,
        line: index + 1,
        type: 'UNSTABLE_DEPS',
        description: 'Hook with potentially unstable dependencies',
        code: line.trim()
      });
    }
  });
}

// Main execution
const srcDir = join(process.cwd(), 'apps/frontend/src');
const files = findTsxFiles(srcDir);

console.log(`Analyzing ${files.length} files...\n`);

files.forEach(analyzeFile);

// Group issues by type
const groupedIssues = issues.reduce((acc, issue) => {
  if (!acc[issue.type]) acc[issue.type] = [];
  acc[issue.type].push(issue);
  return acc;
}, {} as Record<string, Issue[]>);

// Report findings
Object.entries(groupedIssues).forEach(([type, typeIssues]) => {
  console.log(`\n=== ${type} (${typeIssues.length} issues) ===`);
  typeIssues.slice(0, 5).forEach(issue => {
    console.log(`\n${issue.file}:${issue.line}`);
    console.log(`  ${issue.description}`);
    console.log(`  > ${issue.code}`);
  });
  
  if (typeIssues.length > 5) {
    console.log(`\n  ... and ${typeIssues.length - 5} more`);
  }
});

console.log(`\n\nTotal issues found: ${issues.length}`);
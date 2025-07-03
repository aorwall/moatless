import fs from 'fs';
import path from 'path';

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

function findTsxFiles(dir) {
  const files = [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...findTsxFiles(fullPath));
    } else if (entry.isFile() && (entry.name.endsWith('.tsx') || entry.name.endsWith('.jsx'))) {
      files.push(fullPath);
    }
  }
  
  return files;
}

// Run the check
async function checkColors() {
  const files = findTsxFiles('apps/frontend/src');
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
}

checkColors().catch(console.error);
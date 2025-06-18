import { Highlight, themes } from 'prism-react-renderer';
import { useEffect, useState } from 'react';

import type { DiffChunk } from '@/types/git-diff';

import { SyntaxDiffLine } from './syntax-diff-line';

interface SyntaxDiffViewerProps {
  chunk: DiffChunk;
  filePath: string;
}

// Map file extensions to Prism language identifiers
const getLanguageFromPath = (path: string): string => {
  const ext = path.split('.').pop()?.toLowerCase() || '';
  const languageMap: Record<string, string> = {
    js: 'javascript',
    jsx: 'jsx',
    ts: 'typescript',
    tsx: 'tsx',
    py: 'python',
    rb: 'ruby',
    go: 'go',
    java: 'java',
    cpp: 'cpp',
    c: 'c',
    cs: 'csharp',
    php: 'php',
    rs: 'rust',
    kt: 'kotlin',
    swift: 'swift',
    m: 'objectivec',
    scala: 'scala',
    sh: 'bash',
    ps1: 'powershell',
    psm1: 'powershell',
    gradle: 'groovy',
    vim: 'vim',
    lua: 'lua',
    dart: 'dart',
    r: 'r',
    R: 'r',
    jl: 'julia',
    vue: 'vue',
    yaml: 'yaml',
    yml: 'yaml',
    toml: 'toml',
    xml: 'xml',
    html: 'html',
    htm: 'html',
    css: 'css',
    scss: 'scss',
    sass: 'sass',
    less: 'less',
    json: 'json',
    md: 'markdown',
    markdown: 'markdown',
    rst: 'rest',
    tex: 'latex',
    dockerfile: 'docker',
    Dockerfile: 'docker',
    makefile: 'makefile',
    Makefile: 'makefile',
    sql: 'sql',
    graphql: 'graphql',
    gql: 'graphql',
  };

  return languageMap[ext] || 'text';
};

export function SyntaxDiffViewer({ chunk, filePath }: SyntaxDiffViewerProps) {
  const [isDarkMode, setIsDarkMode] = useState(false);

  useEffect(() => {
    // Check if dark mode is enabled
    const checkDarkMode = () => {
      setIsDarkMode(document.documentElement.classList.contains('dark'));
    };

    checkDarkMode();

    // Watch for theme changes
    const observer = new MutationObserver(checkDarkMode);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    });

    return () => observer.disconnect();
  }, []);

  const language = getLanguageFromPath(filePath);
  const lines = chunk.content.split('\n');

  // Filter out empty lines but keep track of their indices
  const nonEmptyLines: Array<{ line: string; originalIndex: number }> = [];
  lines.forEach((line, index) => {
    if (line !== '') {
      nonEmptyLines.push({ line, originalIndex: index });
    }
  });

  let oldLineNumber = chunk.start_line_old;
  let newLineNumber = chunk.start_line_new;

  // Extract all lines without diff markers for syntax highlighting
  // We need to preserve empty lines for proper token mapping
  const codeLines = lines.map((line) => {
    if (line === '') {
      return '';
    }
    const char = line[0];
    if (char === '+' || char === '-' || char === ' ') {
      return line.substring(1);
    }
    return line;
  });

  const codeContent = codeLines.join('\n');
  const theme = isDarkMode ? themes.oneDark : themes.github;

  return (
    <div className="diff-content">
      <Highlight theme={theme} code={codeContent} language={language}>
        {({ tokens, getTokenProps }) => {
          // Create a map from original line index to token line
          const tokensByOriginalLine = new Map<number, (typeof tokens)[0]>();
          let tokenLineIndex = 0;

          lines.forEach((line, originalIndex) => {
            if (line !== '') {
              tokensByOriginalLine.set(
                originalIndex,
                tokens[tokenLineIndex] || [],
              );
              tokenLineIndex++;
            }
          });

          const renderedLines = nonEmptyLines.map(({ line, originalIndex }) => {
            const isAddition = line.startsWith('+');
            const isDeletion = line.startsWith('-');

            const currentOldLine = oldLineNumber;
            const currentNewLine = newLineNumber;

            // Update line numbers for next iteration
            if (isAddition) {
              newLineNumber++;
            } else if (isDeletion) {
              oldLineNumber++;
            } else {
              oldLineNumber++;
              newLineNumber++;
            }

            const lineTokens = tokensByOriginalLine.get(originalIndex) || [];

            return (
              <SyntaxDiffLine
                key={`${chunk.header}-line-${originalIndex}`}
                line={line}
                oldLineNumber={currentOldLine}
                newLineNumber={currentNewLine}
                tokens={lineTokens}
                getTokenProps={getTokenProps}
              />
            );
          });

          return <div className="font-mono text-xs">{renderedLines}</div>;
        }}
      </Highlight>
    </div>
  );
}

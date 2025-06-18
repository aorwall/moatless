import type { ReactElement } from 'react';

import { cn } from '@/lib/utils';

interface Token {
  type?: string;
  content: string;
}

interface DiffLineProps {
  line: string;
  oldLineNumber: number;
  newLineNumber: number;
  tokens: Array<Token>;
  getTokenProps: (props: { token: Token; key?: number }) => {
    className?: string;
    style?: React.CSSProperties;
    key?: string | number;
  };
}

interface LineNumberProps {
  lineNumber: number | null;
  className?: string;
}

function LineNumber({ lineNumber, className }: LineNumberProps) {
  return (
    <span
      className={cn(
        'line-number w-12 px-2 py-1 text-right select-none',
        'text-gray-500 dark:text-gray-400',
        className,
      )}
    >
      {lineNumber || ''}
    </span>
  );
}

interface TokenContentProps {
  tokens: Array<Token>;
  line: string;
  isAddition: boolean;
  isDeletion: boolean;
  getTokenProps: (props: { token: Token; key?: number }) => {
    className?: string;
    style?: React.CSSProperties;
    key?: string | number;
  };
}

function TokenContent({
  tokens,
  line,
  isAddition,
  isDeletion,
  getTokenProps,
}: TokenContentProps) {
  const colorClass = cn(
    isAddition && 'text-green-700 dark:text-green-300',
    isDeletion && 'text-red-700 dark:text-red-300',
  );

  if (tokens.length === 0) {
    return <span className={colorClass}>{line.substring(1)}</span>;
  }

  return (
    <span>
      {tokens.map((token, tokenIndex) => {
        const tokenProps = getTokenProps({ token, key: tokenIndex });
        // Don't apply diff colors to syntax-highlighted tokens
        // Let Prism's syntax colors take precedence
        return (
          <span key={`token-${tokenIndex}-${token.content}`} {...tokenProps}>
            {token.content}
          </span>
        );
      })}
    </span>
  );
}

export function SyntaxDiffLine({
  line,
  oldLineNumber,
  newLineNumber,
  tokens,
  getTokenProps,
}: DiffLineProps): ReactElement {
  const isAddition = line.startsWith('+');
  const isDeletion = line.startsWith('-');
  const isContext = !(isAddition || isDeletion);

  let currentOldLine: number | null = oldLineNumber;
  let currentNewLine: number | null = newLineNumber;

  if (isAddition) {
    currentOldLine = null;
  } else if (isDeletion) {
    currentNewLine = null;
  }

  return (
    <div
      className={cn(
        'diff-line flex group',
        isAddition && 'bg-green-50/50 dark:bg-green-900/10',
        isDeletion && 'bg-red-50/50 dark:bg-red-900/10',
      )}
    >
      <div className="line-numbers flex shrink-0">
        <LineNumber
          lineNumber={currentOldLine}
          className={isDeletion ? 'bg-red-100 dark:bg-red-900/30' : ''}
        />
        <LineNumber
          lineNumber={currentNewLine}
          className={isAddition ? 'bg-green-100 dark:bg-green-900/30' : ''}
        />
      </div>
      <div
        className={cn(
          'line-content flex-1 px-4 py-1 whitespace-pre overflow-x-auto font-mono text-xs',
        )}
      >
        <span
          className={cn(
            'select-none mr-2',
            isAddition && 'text-green-700 dark:text-green-300',
            isDeletion && 'text-red-700 dark:text-red-300',
            isContext && 'text-gray-500 dark:text-gray-400',
          )}
        >
          {isAddition && '+'}
          {isDeletion && '-'}
          {isContext && ' '}
        </span>
        <TokenContent
          tokens={tokens}
          line={line}
          isAddition={isAddition}
          isDeletion={isDeletion}
          getTokenProps={getTokenProps}
        />
      </div>
    </div>
  );
}

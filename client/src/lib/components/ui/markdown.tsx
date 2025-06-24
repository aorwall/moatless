import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { cn } from '@/lib/utils';

interface MarkdownProps {
  content: string;
  className?: string;
}

export function Markdown({ content, className }: MarkdownProps) {
  return (
    <div
      className={cn(
        'prose prose-sm dark:prose-invert max-w-none',
        'prose-p:my-2',
        'prose-pre:bg-gray-100 prose-pre:dark:bg-gray-800',
        'prose-pre:text-gray-800 prose-pre:dark:text-gray-200',
        'prose-code:bg-gray-100 prose-code:dark:bg-gray-800',
        'prose-code:text-gray-800 prose-code:dark:text-gray-200',
        'prose-code:px-1 prose-code:py-0.5 prose-code:rounded',
        'prose-code:before:content-none prose-code:after:content-none',
        'prose-strong:text-current',
        'prose-a:text-blue-600 prose-a:dark:text-blue-400',
        'prose-a:no-underline prose-a:hover:underline',
        'prose-ul:my-2 prose-ol:my-2',
        'prose-li:my-0',
        className,
      )}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
        pre: ({ children, ...props }) => (
          <pre
            className="overflow-x-auto rounded-md bg-gray-100 dark:bg-gray-800 p-4 text-sm"
            {...props}
          >
            {children}
          </pre>
        ),
        code: ({ children, className, ...props }) => {
          const isInline = !className;
          return isInline ? (
            <code
              className="px-1 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-gray-800 dark:text-gray-200 text-sm"
              {...props}
            >
              {children}
            </code>
          ) : (
            <code className={className} {...props}>
              {children}
            </code>
          );
        },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
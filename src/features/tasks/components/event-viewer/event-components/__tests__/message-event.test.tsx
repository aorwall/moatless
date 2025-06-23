import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { Event } from '@/types/event';

import { MessageEvent } from '../message-event';

describe('MessageEvent', () => {
  const createMessageEvent = (payload: Record<string, unknown>): Event => ({
    event_id: 'test-event',
    type: 'message',
    sub_type: '',
    task_id: 'test-task',
    iteration_number: 1,
    timestamp: '2025-01-01T00:00:00Z',
    is_error: false,
    context_tokens: 100,
    num_turns: 1,
    total_cost: 0.01,
    payload,
  });

  it('should render assistant message with bot icon', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [
          {
            type: 'text',
            text: 'Hello from assistant',
          },
        ],
      },
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('Assistant')).toBeInTheDocument();
    expect(screen.getByText('Hello from assistant')).toBeInTheDocument();
    // Bot icon is rendered
    const botIcon = document.querySelector('svg.lucide-bot');
    expect(botIcon).toBeInTheDocument();
  });

  it('should render user message with user icon', () => {
    const event = createMessageEvent({
      type: 'user',
      message: {
        content: [
          {
            type: 'text',
            text: 'Hello from user',
          },
        ],
      },
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('User')).toBeInTheDocument();
    expect(screen.getByText('Hello from user')).toBeInTheDocument();
    // User icon is rendered
    const userIcon = document.querySelector('svg.lucide-user');
    expect(userIcon).toBeInTheDocument();
  });

  it('should render markdown content', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [
          {
            type: 'text',
            text: '# Heading\n\nThis is **bold** text with `inline code`',
          },
        ],
      },
    });

    render(<MessageEvent event={event} />);

    // Markdown is rendered
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Heading');
    expect(screen.getByText('bold')).toBeInTheDocument();
    expect(screen.getByText('inline code')).toBeInTheDocument();
  });

  it('should handle string content directly', () => {
    const event = createMessageEvent({
      type: 'user',
      message: {
        content: 'Direct string content',
      },
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('Direct string content')).toBeInTheDocument();
  });

  it('should handle message.text field', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        text: 'Text field content',
      },
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('Text field content')).toBeInTheDocument();
  });

  it('should handle message.message field', () => {
    const event = createMessageEvent({
      type: 'user',
      message: {
        message: 'Message field content',
      },
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('Message field content')).toBeInTheDocument();
  });

  it('should show no content message when text is empty', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [],
      },
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('No text content')).toBeInTheDocument();
  });

  it('should show no message content when message is null', () => {
    const event = createMessageEvent({
      type: 'assistant',
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('No message content')).toBeInTheDocument();
  });

  it('should handle code blocks in markdown', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [
          {
            type: 'text',
            text: 'Here is some code:\n\n```javascript\nconst hello = "world";\n```',
          },
        ],
      },
    });

    render(<MessageEvent event={event} />);

    expect(screen.getByText('const hello = "world";')).toBeInTheDocument();
    // Code block should be in a pre element
    const preElement = screen.getByText('const hello = "world";').closest('pre');
    expect(preElement).toHaveClass('bg-muted', 'rounded-md', 'p-3');
  });
});
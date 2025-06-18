import type { ParsedEvent } from '../../hooks/use-claude-code-parser';
import { EventRenderer } from './event-renderer';

interface EventItemProps {
  event: ParsedEvent;
}

const getEventStyling = (
  type: ParsedEvent['type'],
  isError: boolean,
): string => {
  const baseStyle =
    'bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700';

  if (isError || type === 'error') {
    return `${baseStyle} border-red-300 dark:border-red-700`;
  }

  return baseStyle;
};

export function EventItem({ event }: EventItemProps) {
  return (
    <div
      className={`${getEventStyling(event.type, event.is_error)} shadow-sm p-4`}
    >
      <EventRenderer event={event} />
    </div>
  );
}

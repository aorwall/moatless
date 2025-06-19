import { Send } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { FileAutocomplete } from '@/features/tasks/components/file-autocomplete';
import { Button } from '@/lib/components/ui/button';
import { Textarea } from '@/lib/components/ui/textarea';
import { useUniversalFileAutocomplete } from '@/lib/hooks/use-universal-file-autocomplete';

interface ChatFormProps {
  message: string;
  onMessageChange: (message: string) => void;
  onSubmit: (e: React.FormEvent) => void;
  textareaDisabled?: boolean;
  sendDisabled?: boolean;
  placeholder?: string;
  environmentId?: string;
  taskId?: string;
  isTaskContext: boolean;
  buttonText: string;
}

export function ChatForm({
  message,
  onMessageChange,
  onSubmit,
  textareaDisabled = false,
  sendDisabled = false,
  placeholder,
  environmentId,
  taskId,
  isTaskContext,
  buttonText,
}: ChatFormProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dropdownPosition, setDropdownPosition] = useState<'above' | 'below'>(
    'below',
  );

  // File autocomplete functionality
  const {
    inputRef,
    isOpen: isAutocompleteOpen,
    results: autocompleteResults,
    selectedIndex,
    isLoading: isAutocompleteLoading,
    error: autocompleteError,
    handleInputChange,
    handleKeyDown: handleAutocompleteKeyDown,
    selectFile,
  } = useUniversalFileAutocomplete({
    environmentId,
    taskId,
    value: message,
    onValueChange: onMessageChange,
  });

  // Detect if dropdown should appear above or below
  useEffect(() => {
    if (isAutocompleteOpen && containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect();
      const spaceBelow = window.innerHeight - rect.bottom;
      const dropdownHeight = 240; // max-h-60 = 15rem = 240px

      // If not enough space below and we're in task context (likely at bottom), show above
      if (spaceBelow < dropdownHeight && isTaskContext) {
        setDropdownPosition('above');
      } else {
        setDropdownPosition('below');
      }
    }
  }, [isAutocompleteOpen, isTaskContext]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Let autocomplete handle key events first
    if (isAutocompleteOpen) {
      handleAutocompleteKeyDown(e);
      if (['ArrowDown', 'ArrowUp', 'Enter', 'Escape', 'Tab'].includes(e.key)) {
        return;
      }
    }

    if (
      e.key === 'Enter' &&
      !e.shiftKey &&
      !isAutocompleteOpen &&
      !sendDisabled
    ) {
      e.preventDefault();
      onSubmit(e as unknown as React.FormEvent);
    }
  };

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const value = e.target.value;
    const cursorPosition = e.target.selectionStart || 0;
    onMessageChange(value);
    handleInputChange(value, cursorPosition);
  };

  // Remove this line as we now use separate disabled states

  return (
    <div className="relative" ref={containerRef}>
      <Textarea
        ref={inputRef}
        placeholder={placeholder}
        value={message}
        onChange={handleTextareaChange}
        onKeyDown={handleKeyDown}
        className="resize-none"
        rows={isTaskContext ? 3 : 4}
        disabled={textareaDisabled}
      />
      {isAutocompleteOpen && (
        <FileAutocomplete
          results={autocompleteResults}
          selectedIndex={selectedIndex}
          onSelect={selectFile}
          isLoading={isAutocompleteLoading}
          error={autocompleteError}
          position={dropdownPosition}
        />
      )}
      <div className="flex justify-end mt-2">
        <Button
          type="submit"
          disabled={sendDisabled}
          className={isTaskContext ? 'self-end' : 'w-full'}
          size={isTaskContext ? 'icon' : 'default'}
          onClick={onSubmit}
        >
          <Send className={`h-4 w-4 ${!isTaskContext ? 'mr-2' : ''}`} />
          {!isTaskContext && <span>{buttonText}</span>}
          {isTaskContext && <span className="sr-only">{buttonText}</span>}
        </Button>
      </div>
    </div>
  );
}

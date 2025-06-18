import { useNavigate } from '@tanstack/react-router';
import { Send } from 'lucide-react';
import { useState } from 'react';
import { toast } from 'sonner';

import { useEnvironmentsList } from '@/features/environments/hooks/use-environments';
import { FileAutocomplete } from '@/features/sessions/components/file-autocomplete';
import { useCreateSession } from '@/features/sessions/hooks/use-sessions';
import { Button } from '@/lib/components/ui/button';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select';
import { Textarea } from '@/lib/components/ui/textarea';
import { useFileAutocomplete } from '@/lib/hooks/use-file-autocomplete';
import { getErrorMessage } from '@/lib/utils/error';

export function SearchSection() {
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedEnvironmentId, setSelectedEnvironmentId] = useState<
    string | undefined
  >();
  const navigate = useNavigate();

  const {
    inputRef,
    isOpen: isAutocompleteOpen,
    results: autocompleteResults,
    selectedIndex,
    isLoading: isAutocompleteLoading,
    handleInputChange,
    handleKeyDown: handleAutocompleteKeyDown,
    selectFile,
  } = useFileAutocomplete({
    environmentId: selectedEnvironmentId,
    value: searchQuery,
    onValueChange: setSearchQuery,
  });

  const { data: environments, isLoading: environmentsLoading } =
    useEnvironmentsList();

  const createSession = useCreateSession({
    onSuccess: (response) => {
      setSearchQuery('');
      navigate({ to: `/sessions/${response.session_id}` });
    },
    onError: (error) => {
      console.error('Failed to create session:', error);
      toast.error('Failed to create session', {
        description: getErrorMessage(error),
      });
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (
      !(searchQuery.trim() && selectedEnvironmentId) ||
      createSession.isPending
    ) {
      return;
    }

    createSession.mutate({
      agent_type: 'claude_code',
      message: searchQuery,
      environment_id: selectedEnvironmentId,
    });
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Let autocomplete handle key events first
    if (isAutocompleteOpen) {
      handleAutocompleteKeyDown(e);
      if (['ArrowDown', 'ArrowUp', 'Enter', 'Escape', 'Tab'].includes(e.key)) {
        return;
      }
    }

    if (e.key === 'Enter' && !e.shiftKey && !isAutocompleteOpen) {
      e.preventDefault();
      handleSubmit(e as unknown as React.FormEvent);
    }
  };

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const value = e.target.value;
    const cursorPosition = e.target.selectionStart || 0;
    setSearchQuery(value);
    handleInputChange(value, cursorPosition);
  };

  return (
    <div className="text-center space-y-4">
      <h1 className="text-4xl font-bold tracking-tight">
        What should we build today?
      </h1>
      <form onSubmit={handleSubmit} className="max-w-2xl mx-auto space-y-4">
        <div className="relative">
          <Textarea
            ref={inputRef}
            placeholder="Describe your coding task or feature... (use @ to reference files)"
            value={searchQuery}
            onChange={handleTextareaChange}
            onKeyDown={handleKeyDown}
            className="resize-none"
            rows={4}
            disabled={createSession.isPending}
          />
          {isAutocompleteOpen && (
            <FileAutocomplete
              results={autocompleteResults}
              selectedIndex={selectedIndex}
              onSelect={selectFile}
              isLoading={isAutocompleteLoading}
            />
          )}
        </div>
        <div className="flex flex-col sm:flex-row gap-4">
          <Select
            value={selectedEnvironmentId}
            onValueChange={setSelectedEnvironmentId}
            disabled={createSession.isPending || environmentsLoading}
          >
            <SelectTrigger className="w-full sm:w-64">
              <SelectValue placeholder="Select environment (required)" />
            </SelectTrigger>
            <SelectContent>
              {environments?.map((env) => (
                <SelectItem key={env.id} value={env.id}>
                  {env.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            type="submit"
            disabled={
              createSession.isPending ||
              !searchQuery.trim() ||
              !selectedEnvironmentId
            }
            className="w-full sm:w-auto"
          >
            <Send className="mr-2 h-4 w-4" />
            {createSession.isPending ? 'Starting...' : 'Start Session'}
          </Button>
        </div>
      </form>
    </div>
  );
}

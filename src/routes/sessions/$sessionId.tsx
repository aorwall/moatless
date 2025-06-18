import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

import { DiffViewer } from '@/features/sessions/components/diff-viewer';
import { LogsViewer } from '@/features/sessions/components/logs-viewer';
import { SessionHeader } from '@/features/sessions/components/session-header';
import {
  useSession,
  useSessionEvents,
  useSessionGitDiff,
} from '@/features/sessions/hooks/use-sessions';
import { useSessionSSE } from '@/features/sessions/hooks/use-sse';
import type { ClaudeMessage } from '@/types/session';

export const Route = createFileRoute('/sessions/$sessionId')({
  component: SessionDetail,
});

function SessionDetail() {
  const { sessionId } = Route.useParams();
  const [, setLocalMessages] = useState<Array<ClaudeMessage>>([]);
  const [streamingEnabled, setStreamingEnabled] = useState(false);

  // Use React Query hooks
  const { data: metadata, isLoading: isLoadingMetadata } =
    useSession(sessionId);
  const { data: events, isLoading: isLoadingEvents } =
    useSessionEvents(sessionId);
  const { data: gitDiff, isLoading: isLoadingGitDiff } =
    useSessionGitDiff(sessionId);

  const loading = isLoadingMetadata || isLoadingEvents || isLoadingGitDiff;

  // Use SSE for real-time updates only if session exists and streaming is enabled
  const { connect, disconnect, isConnected } = useSessionSSE({
    sessionId,
    autoConnect: streamingEnabled && !!metadata && metadata.status === 'active',
    onMessage: (newMessage) => {
      // Add message to local state for immediate UI update
      setLocalMessages((prev) => [...prev, newMessage as ClaudeMessage]);
    },
    onError: (error) => {
      console.error('SSE error:', error);
    },
  });

  // Handle streaming toggle
  const handleStreamingToggle = () => {
    if (streamingEnabled) {
      disconnect();
      setStreamingEnabled(false);
    } else {
      setStreamingEnabled(true);
      if (metadata && metadata.status === 'active') {
        connect();
      }
    }
  };

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        Loading session...
      </div>
    );
  }

  if (!metadata) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        Session not found
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen">
      <SessionHeader
        metadata={metadata}
        streamingEnabled={streamingEnabled}
        isConnected={isConnected}
        onStreamingToggle={handleStreamingToggle}
      />
      <div className="flex flex-1 overflow-hidden">
        {/* Left side - Logs Viewer */}
        <div className="w-1/2 border-r">
          <div className="h-full flex flex-col">
            <div className="border-b px-4 py-2">
              <h2 className="text-sm font-medium">Logs</h2>
            </div>
            <div className="flex-1 overflow-hidden">
              <LogsViewer events={events || []} />
            </div>
          </div>
        </div>

        {/* Right side - Diff Viewer */}
        <div className="w-1/2">
          <div className="h-full flex flex-col">
            <div className="border-b px-4 py-2">
              <h2 className="text-sm font-medium">Diff</h2>
            </div>
            <div className="flex-1 overflow-hidden">
              <DiffViewer gitDiff={gitDiff} />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

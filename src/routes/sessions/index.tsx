import { createFileRoute } from '@tanstack/react-router';

import { SessionList } from '@/features/sessions/components/session-list';
import { useSessionsList } from '@/features/sessions/hooks/use-sessions';

export const Route = createFileRoute('/sessions/')({
  component: SessionsIndex,
});

function SessionsIndex() {
  // Fetch all sessions
  const { data: sessions, isLoading } = useSessionsList(
    {},
    {
      refetchInterval: 5000,
      refetchIntervalInBackground: true,
    },
  );

  // Transform and separate sessions
  const transformedSessions =
    sessions?.map((metadata) => ({
      id: metadata.session_id,
      title: `Session ${metadata.session_id.slice(0, 8)}`,
      time: new Date(metadata.last_message_at).toLocaleString(),
      project: metadata.agent_type,
      status: metadata.status,
      changes: { added: 0, removed: 0 },
    })) || [];

  const activeSessions = transformedSessions.filter(
    (session) => session.status === 'active',
  );

  const finishedSessions = transformedSessions.filter(
    (session) => session.status !== 'active',
  );

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">All Sessions</h1>
          <p className="text-muted-foreground mt-2">
            View and manage all your active and archived sessions
          </p>
        </div>
        <SessionList
          activeSessions={activeSessions}
          archivedSessions={finishedSessions}
          loading={isLoading}
        />
      </div>
    </div>
  );
}

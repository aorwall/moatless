import { createFileRoute } from '@tanstack/react-router';

import { SearchSection } from '@/features/sessions/components/search-section';
import { SessionList } from '@/features/sessions/components/session-list';
import { useSessionsList } from '@/features/sessions/hooks/use-sessions';

export const Route = createFileRoute('/')({
  component: Dashboard,
});

function Dashboard() {
  // Fetch all sessions (both active and finished)
  const { data: sessions, isLoading } = useSessionsList(
    {}, // Remove activeOnly filter to get all sessions
    {
      refetchInterval: 5000, // Poll every 5 seconds
      refetchIntervalInBackground: true,
    },
  );

  // Transform and separate sessions into active and finished
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
      <div className="mx-auto max-w-4xl space-y-8">
        <SearchSection />
        <SessionList
          activeSessions={activeSessions}
          archivedSessions={finishedSessions}
          loading={isLoading}
        />
      </div>
    </div>
  );
}

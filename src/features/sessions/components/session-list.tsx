import { Archive, FileText } from 'lucide-react';

import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@/lib/components/ui/tabs';
import type { Session } from '@/types/session';

import { SessionCard } from './session-card';

interface SessionListProps {
  activeSessions: Array<Session>;
  archivedSessions: Array<Session>;
  loading?: boolean;
}

export function SessionList({
  activeSessions,
  archivedSessions,
  loading,
}: SessionListProps) {
  return (
    <Tabs defaultValue="sessions" className="w-full">
      <TabsList className="grid w-full grid-cols-2">
        <TabsTrigger value="sessions" className="space-x-2">
          <FileText className="h-4 w-4" />
          <span>Active Sessions</span>
        </TabsTrigger>
        <TabsTrigger value="archive" className="space-x-2">
          <Archive className="h-4 w-4" />
          <span>Finished</span>
        </TabsTrigger>
      </TabsList>

      <TabsContent value="sessions" className="space-y-4">
        {loading ? (
          <div className="text-center py-8 text-muted-foreground">
            Loading sessions...
          </div>
        ) : activeSessions.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">
            No active sessions
          </div>
        ) : (
          activeSessions.map((session) => (
            <SessionCard key={session.id} session={session} />
          ))
        )}
      </TabsContent>

      <TabsContent value="archive" className="space-y-4">
        {archivedSessions.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">
            No finished sessions
          </div>
        ) : (
          archivedSessions.map((session) => (
            <SessionCard key={session.id} session={session} />
          ))
        )}
      </TabsContent>
    </Tabs>
  );
}

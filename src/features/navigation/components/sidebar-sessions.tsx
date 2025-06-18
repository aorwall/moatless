import { Link } from '@tanstack/react-router';
import { Activity, Clock } from 'lucide-react';

import { useSessionsList } from '@/features/sessions/hooks/use-sessions';
import { Badge } from '@/lib/components/ui/badge';
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar';

export function SidebarSessions() {
  // Fetch all sessions
  const { data: sessions, isLoading } = useSessionsList(
    {},
    {
      refetchInterval: 5000,
      refetchIntervalInBackground: true,
    },
  );

  // Separate active and recent sessions
  const activeSessions =
    sessions?.filter((s) => s.status === 'active').slice(0, 10) || [];
  const recentSessions =
    sessions
      ?.filter((s) => s.status !== 'active')
      .sort(
        (a, b) =>
          new Date(b.last_message_at).getTime() -
          new Date(a.last_message_at).getTime(),
      )
      .slice(0, 10) || [];

  if (isLoading) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>Sessions</SidebarGroupLabel>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-muted-foreground">
            Loading sessions...
          </div>
        </SidebarGroupContent>
      </SidebarGroup>
    );
  }

  return (
    <>
      {/* Active Sessions */}
      {activeSessions.length > 0 && (
        <SidebarGroup>
          <SidebarGroupLabel>
            <Activity className="mr-2 h-4 w-4" />
            Active Sessions
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {activeSessions.map((session) => (
                <SidebarMenuItem key={session.session_id}>
                  <SidebarMenuButton asChild>
                    <Link to={`/sessions/${session.session_id}`}>
                      <div className="flex flex-col items-start gap-1 overflow-hidden">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-sm">
                            {session.agent_type}
                          </span>
                          <Badge
                            variant="outline"
                            className="h-5 px-1.5 text-xs"
                          >
                            Active
                          </Badge>
                        </div>
                        <span className="truncate text-xs text-muted-foreground">
                          {session.session_id.slice(0, 8)}
                        </span>
                      </div>
                    </Link>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      )}

      {/* Recent Sessions */}
      {recentSessions.length > 0 && (
        <SidebarGroup>
          <SidebarGroupLabel>
            <Clock className="mr-2 h-4 w-4" />
            Recent Sessions
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {recentSessions.map((session) => (
                <SidebarMenuItem key={session.session_id}>
                  <SidebarMenuButton asChild>
                    <Link to={`/sessions/${session.session_id}`}>
                      <div className="flex flex-col items-start gap-1 overflow-hidden">
                        <span className="truncate text-sm">
                          {session.agent_type}
                        </span>
                        <span className="truncate text-xs text-muted-foreground">
                          {new Date(
                            session.last_message_at,
                          ).toLocaleDateString()}
                        </span>
                      </div>
                    </Link>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      )}

      {/* Empty state */}
      {activeSessions.length === 0 && recentSessions.length === 0 && (
        <SidebarGroup>
          <SidebarGroupLabel>Sessions</SidebarGroupLabel>
          <SidebarGroupContent>
            <div className="px-3 py-2 text-sm text-muted-foreground">
              No sessions found
            </div>
          </SidebarGroupContent>
        </SidebarGroup>
      )}
    </>
  );
}

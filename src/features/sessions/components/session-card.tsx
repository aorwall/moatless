import { Link } from '@tanstack/react-router';
import { Clock, GitBranch, Minus, Plus } from 'lucide-react';

import { Badge } from '@/lib/components/ui/badge';
import { Card, CardContent } from '@/lib/components/ui/card';
import type { Session } from '@/types/session';

interface SessionCardProps {
  session: Session;
}

export function SessionCard({ session }: SessionCardProps) {
  return (
    <Link to="/sessions/$sessionId" params={{ sessionId: session.id }}>
      <Card
        className={`cursor-pointer transition-colors hover:bg-muted/50 ${
          session.status === 'error' ? 'opacity-75 hover:opacity-100' : ''
        }`}
      >
        <CardContent className="p-6">
          <div className="flex items-start justify-between space-x-4">
            <div className="space-y-2 flex-1">
              <h3 className="text-lg font-semibold leading-none tracking-tight">
                {session.title}
              </h3>
              <div className="flex items-center space-x-4 text-sm text-muted-foreground">
                <div className="flex items-center space-x-1">
                  <Clock className="h-4 w-4" />
                  <span>{session.time}</span>
                </div>
                <div className="flex items-center space-x-1">
                  <GitBranch className="h-4 w-4" />
                  <span>{session.project}</span>
                </div>
              </div>
            </div>
            <div className="flex items-center space-x-2">
              {session.changes && (
                <>
                  <Badge variant="outline" className="space-x-1">
                    <Plus className="h-3 w-3" />
                    <span>{session.changes.added}</span>
                  </Badge>
                  <Badge variant="outline" className="space-x-1">
                    <Minus className="h-3 w-3" />
                    <span>{session.changes.removed}</span>
                  </Badge>
                </>
              )}
              {session.status === 'active' && (
                <Badge variant="default">Active</Badge>
              )}
              {session.status === 'completed' && (
                <Badge variant="secondary">Completed</Badge>
              )}
              {session.status === 'error' && (
                <Badge variant="destructive">Error</Badge>
              )}
            </div>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

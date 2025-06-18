import { Link, useLocation, useParams } from '@tanstack/react-router';
import { ChevronRight } from 'lucide-react';
import { useMemo } from 'react';

import { useSession } from '@/features/sessions/hooks/use-sessions';
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from '@/lib/components/ui/breadcrumb';

// Helper function to get breadcrumb label
const getBreadcrumbLabel = (
  segment: string,
  prevSegment: string | undefined,
  isSessionId: boolean,
  session: { agent_type: string } | undefined,
): string => {
  const settingsMap: Record<string, string> = {
    environments: 'Environments',
    general: 'General',
    'claude-code': 'Claude Code',
    'data-controls': 'Data Controls',
  };

  if (prevSegment === 'settings' && settingsMap[segment]) {
    return settingsMap[segment];
  }

  if (isSessionId && session) {
    return `${session.agent_type} - ${segment.slice(0, 8)}`;
  }

  if (isSessionId) {
    return segment.slice(0, 8);
  }

  return segment.charAt(0).toUpperCase() + segment.slice(1);
};

export function AppBreadcrumbs() {
  const location = useLocation();
  const params = useParams({ strict: false });

  // Get session data if we're on a session page
  const sessionId = params.sessionId as string | undefined;
  const { data: session } = useSession(sessionId || '', {
    enabled: !!sessionId,
  });

  const breadcrumbs = useMemo(() => {
    const path = location.pathname;
    const segments = path.split('/').filter(Boolean);

    const items: Array<{ label: string; href?: string; key: string }> = [
      { label: 'Home', href: '/', key: 'home' },
    ];

    if (segments.length === 0) {
      return items;
    }

    // Build breadcrumb items based on path segments
    segments.forEach((segment, index) => {
      const currentPath = `/${segments.slice(0, index + 1).join('/')}`;
      const isLast = index === segments.length - 1;
      const prevSegment = segments[index - 1];
      const isSessionId = sessionId ? segment === sessionId : false;

      const label = getBreadcrumbLabel(
        segment,
        prevSegment,
        isSessionId,
        session,
      );

      if (segment === 'sessions') {
        items.push({
          label: 'Sessions',
          href: isLast ? undefined : '/sessions',
          key: `sessions-${index}`,
        });
      } else if (segment === 'settings') {
        items.push({
          label: 'Settings',
          href: isLast ? undefined : '/settings/general',
          key: `settings-${index}`,
        });
      } else {
        items.push({
          label,
          href: isLast ? undefined : currentPath,
          key: `${segment}-${index}`,
        });
      }
    });

    return items;
  }, [location.pathname, session, sessionId]);

  if (breadcrumbs.length <= 1) {
    return null; // Don't show breadcrumbs on home page
  }

  return (
    <Breadcrumb>
      <BreadcrumbList>
        {breadcrumbs.map((item, index) => (
          <div key={item.key} className="contents">
            <BreadcrumbItem>
              {!item.href ? (
                <BreadcrumbPage>{item.label}</BreadcrumbPage>
              ) : (
                <BreadcrumbLink asChild>
                  <Link to={item.href}>{item.label}</Link>
                </BreadcrumbLink>
              )}
            </BreadcrumbItem>
            {index < breadcrumbs.length - 1 && (
              <BreadcrumbSeparator>
                <ChevronRight className="h-3.5 w-3.5" />
              </BreadcrumbSeparator>
            )}
          </div>
        ))}
      </BreadcrumbList>
    </Breadcrumb>
  );
}

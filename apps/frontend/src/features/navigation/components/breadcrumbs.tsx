import { Link, useLocation, useParams } from '@tanstack/react-router'
import { ChevronRight } from 'lucide-react'
import { useMemo } from 'react'

import { useTask } from '@/features/tasks/hooks/use-tasks'
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from '@/lib/components/ui/breadcrumb'

// Helper function to get breadcrumb label
const getBreadcrumbLabel = (
  segment: string,
  prevSegment: string | undefined,
  isTaskId: boolean,
  task: { agentType: string } | undefined,
): string => {
  const settingsMap: Record<string, string> = {
    environments: 'Environments',
    general: 'General',
    'claude-code': 'Claude Code',
    'data-controls': 'Data Controls',
  }

  if (prevSegment === 'settings' && settingsMap[segment]) {
    return settingsMap[segment]
  }

  if (isTaskId && task) {
    return `${task.agentType} - ${segment.slice(0, 8)}`
  }

  if (isTaskId) {
    return segment.slice(0, 8)
  }

  return segment.charAt(0).toUpperCase() + segment.slice(1)
}

// Helper function to create breadcrumb item
const createBreadcrumbItem = (
  segment: string,
  index: number,
  segments: string[],
  taskId: string | undefined,
  task: { agentType: string } | undefined,
): { label: string; href?: string; key: string } => {
  const currentPath = `/${segments.slice(0, index + 1).join('/')}`
  const isLast = index === segments.length - 1
  const prevSegment = segments[index - 1]
  const isTaskId = taskId ? segment === taskId : false

  const label = getBreadcrumbLabel(segment, prevSegment, isTaskId, task)

  if (segment === 'tasks') {
    return {
      label: 'Tasks',
      href: isLast ? undefined : '/tasks',
      key: `tasks-${index}`,
    }
  }

  if (segment === 'settings') {
    return {
      label: 'Settings',
      href: isLast ? undefined : '/settings',
      key: `settings-${index}`,
    }
  }

  return {
    label,
    href: isLast ? undefined : currentPath,
    key: `${segment}-${index}`,
  }
}

export function AppBreadcrumbs() {
  const location = useLocation()
  const params = useParams({ strict: false })

  // Get task data if we're on a task page
  const taskId = params['task-id'] as string | undefined
  const { data: task } = useTask(taskId || '', {
    enabled: !!taskId,
  })

  const breadcrumbs = useMemo(() => {
    const path = location.pathname
    const segments = path.split('/').filter(Boolean)

    const items: { label: string; href?: string; key: string }[] = [
      { label: 'Home', href: '/', key: 'home' },
    ]

    if (segments.length === 0) {
      return items
    }

    // Build breadcrumb items based on path segments
    segments.forEach((segment, index) => {
      const item = createBreadcrumbItem(segment, index, segments, taskId, task)
      items.push(item)
    })

    return items
  }, [location.pathname, task, taskId])

  if (breadcrumbs.length <= 1) {
    return null // Don't show breadcrumbs on home page
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
  )
}

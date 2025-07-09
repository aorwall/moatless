import { ChevronRight, File, Folder } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import {
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
} from '@/lib/components/ui/sidebar'
import { useFileList } from '../api/use-file-list'
import type { FileTreeNode } from '../types/file-tree'
import { buildTreeStructure } from '../utils/file-tree-utils'
import { FileTreeSkeleton } from './file-tree-skeleton'

interface FileTreeProps {
  taskId: string
  onFileSelect: (filePath: string) => void
}

export function FileTree({ taskId, onFileSelect }: FileTreeProps) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const { data, isLoading, error } = useFileList({ taskId, recursive: true })

  const tree = useMemo(() => {
    if (!data?.files) return []
    return buildTreeStructure(data.files)
  }, [data?.files])

  const handleFileSelect = (path: string) => {
    setSelectedPath(path)
    onFileSelect(path)
  }

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading) {
    return <FileTreeSkeleton />
  }

  if (!data?.files || data.files.length === 0) {
    return <div className="p-4 text-sm text-muted-foreground text-center">No files found</div>
  }

  return (
    <SidebarContent className="h-full overflow-y-auto">
      <SidebarGroup>
        <SidebarGroupLabel>Files</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            {tree.map((item) => (
              <TreeNode
                key={item.path}
                item={item}
                selectedPath={selectedPath}
                onFileSelect={handleFileSelect}
              />
            ))}
          </SidebarMenu>
        </SidebarGroupContent>
      </SidebarGroup>
    </SidebarContent>
  )
}

function TreeNode({
  item,
  selectedPath,
  onFileSelect,
}: {
  item: FileTreeNode
  selectedPath: string | null
  onFileSelect: (path: string) => void
}) {
  if (item.type === 'file') {
    return (
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={selectedPath === item.path}
          onClick={() => onFileSelect(item.path)}
          className="data-[active=true]:bg-accent"
        >
          <File className="h-4 w-4" />
          {item.name}
        </SidebarMenuButton>
      </SidebarMenuItem>
    )
  }

  return (
    <SidebarMenuItem>
      <Collapsible className="group/collapsible [&[data-state=open]>button>svg:first-child]:rotate-90">
        <CollapsibleTrigger asChild>
          <SidebarMenuButton>
            <ChevronRight className="transition-transform h-4 w-4" />
            <Folder className="h-4 w-4" />
            {item.name}
          </SidebarMenuButton>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <SidebarMenuSub>
            {item.children?.map((subItem) => (
              <TreeNode
                key={subItem.path}
                item={subItem}
                selectedPath={selectedPath}
                onFileSelect={onFileSelect}
              />
            ))}
          </SidebarMenuSub>
        </CollapsibleContent>
      </Collapsible>
    </SidebarMenuItem>
  )
}

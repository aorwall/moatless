import { ChevronRight, File, Folder } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { cn } from '@/lib/utils'
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
    // Extract file paths from the objects
    const filePaths = data.files.map((file) => file.path)
    return buildTreeStructure(filePaths)
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
    <div className="h-full w-full flex flex-col">
      <div className="p-3 border-b flex-shrink-0">
        <h3 className="text-sm font-medium">Files</h3>
      </div>
      <div className="flex-1 overflow-auto">
        <div className="min-w-max">
          {tree.map((item) => (
            <TreeNode
              key={item.path}
              item={item}
              selectedPath={selectedPath}
              onFileSelect={handleFileSelect}
            />
          ))}
        </div>
      </div>
    </div>
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
      <div>
        <button
          type="button"
          onClick={() => onFileSelect(item.path)}
          className={cn(
            'w-full flex items-center gap-2 px-3 py-1.5 text-sm rounded-md hover:bg-muted transition-colors whitespace-nowrap',
            selectedPath === item.path && 'bg-accent',
          )}
        >
          <File className="h-4 w-4 flex-shrink-0" />
          <span className="text-left">{item.name}</span>
        </button>
      </div>
    )
  }

  return (
    <div>
      <Collapsible>
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="w-full flex items-center gap-2 px-3 py-1.5 text-sm rounded-md hover:bg-muted transition-colors whitespace-nowrap"
          >
            <ChevronRight className="h-4 w-4 flex-shrink-0 transition-transform data-[state=open]:rotate-90" />
            <Folder className="h-4 w-4 flex-shrink-0" />
            <span className="text-left">{item.name}</span>
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="pl-4">
            {item.children?.map((subItem) => (
              <TreeNode
                key={subItem.path}
                item={subItem}
                selectedPath={selectedPath}
                onFileSelect={onFileSelect}
              />
            ))}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

import { MultiFileDiff } from '@pierre/diffs/react'
import { usePierreOptions } from '../lib/use-pierre-theme'

interface DiffFileViewProps {
  oldContent: string
  newContent: string
  filename: string
}

export function DiffFileView({ oldContent, newContent, filename }: DiffFileViewProps) {
  const options = usePierreOptions()
  return (
    <MultiFileDiff
      oldFile={{ name: filename, contents: oldContent }}
      newFile={{ name: filename, contents: newContent }}
      options={options}
    />
  )
}

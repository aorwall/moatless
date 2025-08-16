import { FolderGit2, Settings } from 'lucide-react'
import { useState } from 'react'
import { ViewToggle } from '@/components/view-toggle'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

type ViewMode = 'code' | 'vibe'

interface TopBarProps {
  onRepositoriesClick: () => void
  onSettingsClick: () => void
  viewMode: ViewMode
  onViewModeChange: (mode: ViewMode) => void
}

export function TopBar({
  onRepositoriesClick,
  onSettingsClick,
  viewMode,
  onViewModeChange,
}: TopBarProps) {
  const [isMenuOpen, setIsMenuOpen] = useState(false)

  const handleRepositoriesClick = () => {
    onRepositoriesClick()
    setIsMenuOpen(false)
  }

  const handleSettingsClick = () => {
    onSettingsClick()
    setIsMenuOpen(false)
  }

  return (
    <header className="border-b">
      <div className="container mx-auto max-w-7xl flex h-14 items-center px-6">
        <h1 className="font-bold text-lg">Moatless Vibe</h1>
        <div className="ml-auto flex items-center gap-4">
          <ViewToggle mode={viewMode} onModeChange={onViewModeChange} />
          <Popover open={isMenuOpen} onOpenChange={setIsMenuOpen}>
            <PopoverTrigger asChild>
              <Button variant="ghost" size="icon">
                <Settings className="h-5 w-5" />
              </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-48 p-1">
              <div className="space-y-1">
                <Button
                  variant="ghost"
                  className="w-full justify-start h-8 px-2"
                  onClick={handleRepositoriesClick}
                >
                  <FolderGit2 className="mr-2 h-4 w-4" />
                  Repositories
                </Button>
                <Button
                  variant="ghost"
                  className="w-full justify-start h-8 px-2"
                  onClick={handleSettingsClick}
                >
                  <Settings className="mr-2 h-4 w-4" />
                  Settings
                </Button>
              </div>
            </PopoverContent>
          </Popover>
        </div>
      </div>
    </header>
  )
}

import { Link } from '@tanstack/react-router'
import { Box, FolderGit2, LogOut, Settings, User } from 'lucide-react'
import { useState } from 'react'
import { useAuth } from '@/features/auth'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { Separator } from '@/lib/components/ui/separator'

interface TopBarProps {
  onRepositoriesClick: () => void
  onSettingsClick: () => void
}

export function TopBar({ onRepositoriesClick, onSettingsClick }: TopBarProps) {
  const [isMenuOpen, setIsMenuOpen] = useState(false)
  const { user, isAuthenticated, logout } = useAuth()

  const handleRepositoriesClick = () => {
    onRepositoriesClick()
    setIsMenuOpen(false)
  }

  const handleSettingsClick = () => {
    onSettingsClick()
    setIsMenuOpen(false)
  }

  const handleLogout = () => {
    logout()
    setIsMenuOpen(false)
  }

  return (
    <header className="border-b">
      <div className="container mx-auto max-w-7xl flex h-14 items-center px-6">
        <h1 className="font-bold text-lg">Moatless Vibe</h1>
        <div className="ml-auto flex items-center gap-4">
          <Popover open={isMenuOpen} onOpenChange={setIsMenuOpen}>
            <PopoverTrigger asChild>
              <Button variant="ghost" size="icon">
                <Settings className="h-5 w-5" />
              </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-48 p-1">
              <div className="space-y-1">
                <Link to="/sandbox" onClick={() => setIsMenuOpen(false)}>
                  <Button variant="ghost" className="w-full justify-start h-8 px-2">
                    <Box className="mr-2 h-4 w-4" />
                    Sandbox
                  </Button>
                </Link>
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
                {isAuthenticated && user && (
                  <>
                    <Separator className="my-1" />
                    <div className="px-2 py-1.5 text-sm">
                      <div className="flex items-center gap-2">
                        {user.avatarUrl ? (
                          <img
                            src={user.avatarUrl}
                            alt={user.login}
                            className="w-6 h-6 rounded-full"
                          />
                        ) : (
                          <User className="w-4 h-4" />
                        )}
                        <div className="flex-1 min-w-0">
                          <div className="font-medium truncate">{user.login}</div>
                          {user.email && (
                            <div className="text-xs text-muted-foreground truncate">
                              {user.email}
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                    <Button
                      variant="ghost"
                      className="w-full justify-start h-8 px-2 text-destructive hover:text-destructive"
                      onClick={handleLogout}
                    >
                      <LogOut className="mr-2 h-4 w-4" />
                      Logout
                    </Button>
                  </>
                )}
              </div>
            </PopoverContent>
          </Popover>
        </div>
      </div>
    </header>
  )
}

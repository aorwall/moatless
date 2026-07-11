import { Link } from '@tanstack/react-router'
import { AlertTriangle, Box, FolderGit2, LogOut, Settings, Shield, User } from 'lucide-react'
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
  const { user, isAuthenticated, isAdmin, githubConnected, logout } = useAuth()
  const showReconnectGithub = isAuthenticated && !githubConnected

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
          {showReconnectGithub && (
            <Button
              variant="outline"
              size="sm"
              className="h-8 border-amber-500/60 bg-amber-500/10 text-amber-700 hover:bg-amber-500/20 hover:text-amber-800 dark:text-amber-400 dark:hover:text-amber-300"
              onClick={onSettingsClick}
              title="Your GitHub access is unavailable. Click to reconnect."
            >
              <AlertTriangle className="mr-1.5 h-4 w-4" />
              Reconnect GitHub
            </Button>
          )}
          <Popover open={isMenuOpen} onOpenChange={setIsMenuOpen}>
            <PopoverTrigger asChild>
              <Button variant="ghost" size="icon" aria-label="Open main menu">
                <Settings className="h-5 w-5" />
              </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-56 p-1">
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
                {isAdmin && (
                  <Link to="/admin" onClick={() => setIsMenuOpen(false)}>
                    <Button variant="ghost" className="w-full justify-start h-8 px-2">
                      <Shield className="mr-2 h-4 w-4" />
                      Admin
                    </Button>
                  </Link>
                )}
                {isAuthenticated && user && (
                  <>
                    <Separator className="my-1" />
                    <Button
                      variant="ghost"
                      className="w-full justify-start h-auto px-2 py-1.5"
                      onClick={handleSettingsClick}
                    >
                      <div className="flex w-full items-center gap-2">
                        {user.avatarUrl ? (
                          <img
                            src={user.avatarUrl}
                            alt={user.login}
                            className="w-6 h-6 rounded-full"
                          />
                        ) : (
                          <User className="w-4 h-4" />
                        )}
                        <div className="flex-1 min-w-0 text-left">
                          <div className="font-medium truncate">{user.login}</div>
                          {user.email && (
                            <div className="text-xs text-muted-foreground truncate">
                              {user.email}
                            </div>
                          )}
                        </div>
                      </div>
                    </Button>
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

import { Bell, BookOpen, Code2, Settings } from 'lucide-react';

import { Button } from '@/lib/components/ui/button';

export function Header() {
  return (
    <header className="border-b">
      <div className="container flex h-16 items-center justify-between">
        <div className="flex items-center space-x-2">
          <Code2 className="h-6 w-6" />
          <span className="text-xl font-bold">DevFlow</span>
        </div>
        <div className="flex items-center space-x-2">
          <Button variant="ghost" size="sm">
            <Settings className="mr-2 h-4 w-4" />
            Settings
          </Button>
          <Button variant="ghost" size="sm">
            <BookOpen className="mr-2 h-4 w-4" />
            Docs
          </Button>
          <Button variant="ghost" size="icon">
            <Bell className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </header>
  );
}

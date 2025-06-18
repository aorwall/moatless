import { createFileRoute, Outlet, useNavigate } from '@tanstack/react-router';

import { Tabs, TabsList, TabsTrigger } from '@/lib/components/ui/tabs';

export const Route = createFileRoute('/settings')({
  component: SettingsLayout,
});

function SettingsLayout() {
  const navigate = useNavigate();
  const location = window.location.pathname;

  const getCurrentTab = () => {
    if (location.includes('/settings/environments')) {
      return 'environments';
    }
    if (location.includes('/settings/data-controls')) {
      return 'data-controls';
    }
    if (location.includes('/settings/claude-code')) {
      return 'claude-code';
    }
    return 'general';
  };

  const handleTabChange = (value: string) => {
    navigate({ to: `/settings/${value}` });
  };

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">Settings</h1>
          <p className="text-muted-foreground mt-2">
            Manage your application settings and configurations
          </p>
        </div>

        <Tabs
          value={getCurrentTab()}
          onValueChange={handleTabChange}
          className="space-y-6"
        >
          <TabsList className="grid w-full grid-cols-4">
            <TabsTrigger value="general">General</TabsTrigger>
            <TabsTrigger value="environments">Environments</TabsTrigger>
            <TabsTrigger value="data-controls">Data Controls</TabsTrigger>
            <TabsTrigger value="claude-code">Claude Code</TabsTrigger>
          </TabsList>

          <div className="mt-6">
            <Outlet />
          </div>
        </Tabs>
      </div>
    </div>
  );
}

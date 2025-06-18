import { createFileRoute } from '@tanstack/react-router';

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/lib/components/ui/card';

export const Route = createFileRoute('/settings/general')({
  component: GeneralSettings,
});

function GeneralSettings() {
  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>General Settings</CardTitle>
          <CardDescription>
            Configure general application preferences and behavior.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="space-y-4">
            <div className="text-sm text-muted-foreground">
              General settings configuration will be implemented here.
            </div>
            <div className="space-y-2">
              <p className="text-sm font-medium">Coming soon:</p>
              <ul className="text-sm text-muted-foreground space-y-1 ml-4">
                <li>• Application theme preferences</li>
                <li>• Language and localization settings</li>
                <li>• Default workspace configuration</li>
                <li>• Notification preferences</li>
                <li>• Auto-save and backup settings</li>
              </ul>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

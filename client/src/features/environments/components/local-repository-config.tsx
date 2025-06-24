import { Input } from '@/lib/components/ui/input';
import { Label } from '@/lib/components/ui/label';
import { Switch } from '@/lib/components/ui/switch';

interface LocalRepositoryConfigProps {
  path: string;
  useGitWorktrees: boolean;
  onPathChange: (path: string) => void;
  onUseGitWorktreesChange: (useGitWorktrees: boolean) => void;
}

export function LocalRepositoryConfig({
  path,
  useGitWorktrees,
  onPathChange,
  onUseGitWorktreesChange,
}: LocalRepositoryConfigProps) {
  return (
    <div className="space-y-4">
      <div>
        <Label htmlFor="path">Repository Path</Label>
        <Input
          id="path"
          value={path}
          onChange={(e) => onPathChange(e.target.value)}
          placeholder="/path/to/local/repository"
          required
        />
        <p className="text-sm text-muted-foreground mt-1">
          Absolute path to the local repository directory
        </p>
      </div>

      <div className="flex items-center space-x-2">
        <Switch
          id="use_git_worktrees"
          checked={useGitWorktrees}
          onCheckedChange={onUseGitWorktreesChange}
        />
        <Label htmlFor="use_git_worktrees">Use Git Worktrees</Label>
      </div>
      <p className="text-sm text-muted-foreground">
        Create Git worktrees for each session to isolate changes
      </p>
    </div>
  );
}

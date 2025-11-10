import * as LucideIcons from 'lucide-react'
import type React from 'react'
import { useId } from 'react'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

interface CommandUICustomizationProps {
  icon?: string
  variant?: 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive'
  onIconChange: (icon: string) => void
  onVariantChange: (variant: 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive') => void
}

export function CommandUICustomization({
  icon,
  variant,
  onIconChange,
  onVariantChange,
}: CommandUICustomizationProps) {
  const iconId = useId()
  const variantId = useId()

  return (
    <>
      <div className="space-y-2">
        <Label htmlFor={iconId}>Icon</Label>
        <Select
          value={icon || 'none'}
          onValueChange={(value) => onIconChange(value === 'none' ? '' : value)}
        >
          <SelectTrigger id={iconId}>
            <SelectValue placeholder="Select an icon (optional)">
              {icon && (
                <div className="flex items-center gap-2">
                  {(() => {
                    const IconComponent = (
                      LucideIcons as unknown as Record<
                        string,
                        React.ComponentType<{ className?: string }>
                      >
                    )[icon]
                    return IconComponent ? <IconComponent className="h-4 w-4" /> : null
                  })()}
                  <span>{icon}</span>
                </div>
              )}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none">No icon</SelectItem>
            <SelectItem value="GitCommit">
              <div className="flex items-center gap-2">
                <LucideIcons.GitCommit className="h-4 w-4" />
                <span>GitCommit</span>
              </div>
            </SelectItem>
            <SelectItem value="GitBranch">
              <div className="flex items-center gap-2">
                <LucideIcons.GitBranch className="h-4 w-4" />
                <span>GitBranch</span>
              </div>
            </SelectItem>
            <SelectItem value="GitMerge">
              <div className="flex items-center gap-2">
                <LucideIcons.GitMerge className="h-4 w-4" />
                <span>GitMerge</span>
              </div>
            </SelectItem>
            <SelectItem value="GitPullRequest">
              <div className="flex items-center gap-2">
                <LucideIcons.GitPullRequest className="h-4 w-4" />
                <span>GitPullRequest</span>
              </div>
            </SelectItem>
            <SelectItem value="Play">
              <div className="flex items-center gap-2">
                <LucideIcons.Play className="h-4 w-4" />
                <span>Play</span>
              </div>
            </SelectItem>
            <SelectItem value="TestTube">
              <div className="flex items-center gap-2">
                <LucideIcons.TestTube className="h-4 w-4" />
                <span>TestTube</span>
              </div>
            </SelectItem>
            <SelectItem value="Hammer">
              <div className="flex items-center gap-2">
                <LucideIcons.Hammer className="h-4 w-4" />
                <span>Hammer</span>
              </div>
            </SelectItem>
            <SelectItem value="Package">
              <div className="flex items-center gap-2">
                <LucideIcons.Package className="h-4 w-4" />
                <span>Package</span>
              </div>
            </SelectItem>
            <SelectItem value="Bug">
              <div className="flex items-center gap-2">
                <LucideIcons.Bug className="h-4 w-4" />
                <span>Bug</span>
              </div>
            </SelectItem>
            <SelectItem value="Zap">
              <div className="flex items-center gap-2">
                <LucideIcons.Zap className="h-4 w-4" />
                <span>Zap</span>
              </div>
            </SelectItem>
          </SelectContent>
        </Select>
        <p className="text-sm text-muted-foreground">
          Choose an icon to display next to the command button
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor={variantId}>Button Style</Label>
        <Select value={variant || 'default'} onValueChange={onVariantChange}>
          <SelectTrigger id={variantId}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">Default</SelectItem>
            <SelectItem value="outline">Outline</SelectItem>
            <SelectItem value="secondary">Secondary</SelectItem>
            <SelectItem value="ghost">Ghost</SelectItem>
            <SelectItem value="destructive">Destructive</SelectItem>
          </SelectContent>
        </Select>
        <p className="text-sm text-muted-foreground">
          Choose the visual style of the command button
        </p>
      </div>
    </>
  )
}

'use client';

import {
  ChevronDown,
  ChevronRight,
  Minus,
  MoreHorizontal,
  Plus,
} from 'lucide-react';
import { useState } from 'react';

import { Badge } from '@/lib/components/ui/badge';
import { Button } from '@/lib/components/ui/button';
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/lib/components/ui/card';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible';
import type { FileDiff } from '@/types/task';

interface FileDiffProps {
  file: FileDiff;
  defaultOpen?: boolean;
}

export function FileDiffComponent({ file, defaultOpen = true }: FileDiffProps) {
  const [isOpen, setIsOpen] = useState(defaultOpen);

  return (
    <Card>
      <Collapsible open={isOpen} onOpenChange={setIsOpen}>
        <CollapsibleTrigger asChild>
          <CardHeader className="cursor-pointer hover:bg-muted/50">
            <div className="flex items-center justify-between">
              <div className="flex items-center space-x-2">
                {isOpen ? (
                  <ChevronDown className="h-4 w-4" />
                ) : (
                  <ChevronRight className="h-4 w-4" />
                )}
                <CardTitle className="text-base font-mono">
                  {file.name}
                </CardTitle>
                <Badge
                  variant={file.status === 'New' ? 'default' : 'secondary'}
                >
                  {file.status}
                </Badge>
              </div>
              <div className="flex items-center space-x-2">
                <Badge variant="outline" className="space-x-1">
                  <Plus className="h-3 w-3" />
                  <span>{file.changes.added}</span>
                </Badge>
                <Badge variant="outline" className="space-x-1">
                  <Minus className="h-3 w-3" />
                  <span>{file.changes.removed}</span>
                </Badge>
                <Button variant="ghost" size="icon">
                  <MoreHorizontal className="h-4 w-4" />
                </Button>
              </div>
            </div>
          </CardHeader>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <CardContent>
            <pre className="text-sm font-mono whitespace-pre-wrap bg-muted p-4 rounded-md overflow-x-auto">
              {file.diff}
            </pre>
          </CardContent>
        </CollapsibleContent>
      </Collapsible>
    </Card>
  );
}

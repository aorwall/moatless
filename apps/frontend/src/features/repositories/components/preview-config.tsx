import {
  FRAMEWORK_DEFAULT_COMMANDS,
  FRAMEWORK_OPTIONS,
  INJECTION_METHOD_OPTIONS,
} from '@moatless/types'
import type { PreviewConfig, UpdateRepositoryRequest } from '@moatless/types'
import { ChevronDown, Info, Loader2, Wand2 } from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Collapsible, CollapsibleContent } from '@/lib/components/ui/collapsible'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Separator } from '@/lib/components/ui/separator'

interface PreviewConfigFormProps {
  previewConfig?: PreviewConfig
  onUpdate: (previewConfig: UpdateRepositoryRequest['previewConfig']) => void
  isSubmitting: boolean
  repositoryId: string
  onDetect?: () => Promise<PreviewConfig | undefined>
}

export function PreviewConfigForm({
  previewConfig,
  onUpdate,
  isSubmitting,
  onDetect,
}: PreviewConfigFormProps) {
  const [isDetecting, setIsDetecting] = useState(false)
  const [showInjectionDetails, setShowInjectionDetails] = useState(false)
  const [config, setConfig] = useState<PreviewConfig>({
    framework: previewConfig?.framework,
    frameworkVersion: previewConfig?.frameworkVersion || '',
    serverStartCommand: previewConfig?.serverStartCommand || '',
    injectionStrategy: previewConfig?.injectionStrategy || {
      method: 'html-injection',
      config: {},
    },
  })

  const selectedInjectionMethod = INJECTION_METHOD_OPTIONS.find(
    (option) => option.value === (config.injectionStrategy?.method || 'html-injection'),
  )

  const handleDetect = async () => {
    if (!onDetect) return

    setIsDetecting(true)
    try {
      const detected = await onDetect()
      if (detected) {
        setConfig({
          ...config,
          ...detected,
        })
      }
    } finally {
      setIsDetecting(false)
    }
  }

  const handleFrameworkChange = (value: string) => {
    const framework = value as PreviewConfig['framework']

    // Get default command for the selected framework
    const defaultCommand = framework ? FRAMEWORK_DEFAULT_COMMANDS[framework] : ''

    setConfig({
      ...config,
      framework,
      // Set default server command if none is set or if it's the default for another framework
      serverStartCommand:
        !config.serverStartCommand ||
        Object.values(FRAMEWORK_DEFAULT_COMMANDS).includes(config.serverStartCommand as any)
          ? defaultCommand
          : config.serverStartCommand,
    })
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    onUpdate(config)
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle>Preview Configuration</CardTitle>
            <CardDescription>
              Configure how the preview environment works for this repository
            </CardDescription>
          </div>
          {config.framework && (
            <Badge variant="secondary" className="text-xs">
              Auto-detected
            </Badge>
          )}
        </div>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-6">
          {onDetect && (
            <div className="flex justify-end">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handleDetect}
                disabled={isDetecting}
              >
                {isDetecting ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Detecting...
                  </>
                ) : (
                  <>
                    <Wand2 className="mr-2 h-4 w-4" />
                    Auto-detect Configuration
                  </>
                )}
              </Button>
            </div>
          )}

          <div className="space-y-4">
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="framework">Framework</Label>
                <Select value={config.framework} onValueChange={handleFrameworkChange}>
                  <SelectTrigger id="framework">
                    <SelectValue placeholder="Select framework" />
                  </SelectTrigger>
                  <SelectContent>
                    {FRAMEWORK_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="frameworkVersion">Framework Version</Label>
                <Input
                  id="frameworkVersion"
                  placeholder="e.g., 5.0.0"
                  value={config.frameworkVersion}
                  onChange={(e) => setConfig({ ...config, frameworkVersion: e.target.value })}
                />
                <p className="text-xs text-muted-foreground">
                  Optional - helps with compatibility detection
                </p>
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="serverStartCommand">Server Start Command</Label>
              <Input
                id="serverStartCommand"
                placeholder="e.g., npm run dev"
                value={config.serverStartCommand}
                onChange={(e) => setConfig({ ...config, serverStartCommand: e.target.value })}
              />
              <p className="text-sm text-muted-foreground">
                Command to start the development server. Use {'{PORT}'} as a placeholder for the
                port number.
                {config.framework && FRAMEWORK_DEFAULT_COMMANDS[config.framework] && (
                  <span className="block mt-1">
                    Default for{' '}
                    {FRAMEWORK_OPTIONS.find((opt) => opt.value === config.framework)?.label}:{' '}
                    <code className="text-xs bg-muted px-1 py-0.5 rounded">
                      {FRAMEWORK_DEFAULT_COMMANDS[config.framework]}
                    </code>
                  </span>
                )}
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="serverPort">Server Port</Label>
              <Input
                id="serverPort"
                type="number"
                placeholder="3000"
                value={config.serverPort}
                onChange={(e) =>
                  setConfig({ ...config, serverPort: Number.parseInt(e.target.value) || 3000 })
                }
              />
              <p className="text-xs text-muted-foreground">
                Used to replace {'{PORT}'} placeholders in server commands. The system always finds
                and uses random available ports for actual binding.
              </p>
            </div>
        
            <Separator />

            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <h4 className="text-sm font-medium">Script Injection Strategy</h4>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => setShowInjectionDetails(!showInjectionDetails)}
                >
                  <Info className="mr-2 h-4 w-4" />
                  Learn More
                  <ChevronDown
                    className={`ml-2 h-4 w-4 transition-transform ${showInjectionDetails ? 'rotate-180' : ''}`}
                  />
                </Button>
              </div>

              <Collapsible open={showInjectionDetails} onOpenChange={setShowInjectionDetails}>
                <CollapsibleContent className="space-y-4">
                  <div className="rounded-lg border p-4 bg-muted/50">
                    <h5 className="font-medium mb-2">What is Script Injection?</h5>
                    <p className="text-sm text-muted-foreground mb-3">
                      Script injection determines how preview inspector scripts are added to your
                      application. Different methods work better with different frameworks and
                      deployment setups.
                    </p>

                    <div className="space-y-3">
                      {INJECTION_METHOD_OPTIONS.filter((method) =>
                        ['html-injection', 'middleware'].includes(method.value),
                      ).map((method) => (
                        <div key={method.value} className="border rounded p-3">
                          <div className="flex items-start justify-between mb-2">
                            <h6 className="font-medium text-sm">{method.label}</h6>
                            <Badge variant="outline" className="text-xs">
                              {method.value}
                            </Badge>
                          </div>
                          <p className="text-xs text-muted-foreground mb-2">{method.description}</p>

                          {method.value === 'html-injection' ? (
                            <p className="text-xs text-muted-foreground">
                              The system will search for HTML files in this order and inject the
                              preview inspector script into the first one found:
                              <span className="block mt-1 font-mono">
                                index.html → public/index.html → dist/index.html → build/index.html
                              </span>
                              <span className="block mt-1">
                                The script is injected only once into the {'<head>'} tag or at the
                                beginning of {'<body>'}. If you specify a custom path above, it will
                                be checked first.
                              </span>
                            </p>
                          ) : method.value === 'middleware' ? (
                            <p className="text-xs text-muted-foreground">
                              Currently only implemented for Next.js applications. The system
                              creates a middleware file that adds headers to HTML responses,
                              enabling script injection without modifying source files.
                            </p>
                          ) : (
                            <p className="text-xs text-muted-foreground">{method.details}</p>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                </CollapsibleContent>
              </Collapsible>

              <div className="space-y-2">
                <Label htmlFor="injectionMethod">Injection Method</Label>
                <Select
                  value={config.injectionStrategy?.method || 'html-injection'}
                  onValueChange={(value) =>
                    setConfig({
                      ...config,
                      injectionStrategy: {
                        method: value as NonNullable<PreviewConfig['injectionStrategy']>['method'],
                        config: config.injectionStrategy?.config || {},
                      },
                    })
                  }
                >
                  <SelectTrigger id="injectionMethod">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {INJECTION_METHOD_OPTIONS.filter((option) =>
                      ['html-injection', 'middleware'].includes(option.value),
                    ).map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        <div className="flex flex-col">
                          <span>{option.label}</span>
                          <span className="text-xs text-muted-foreground">
                            {option.description}
                          </span>
                        </div>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {selectedInjectionMethod && (
                  <div className="rounded border p-3 bg-muted/30">
                    {selectedInjectionMethod.value === 'html-injection' ? (
                      <p className="text-xs text-muted-foreground">
                        The system will search for HTML files in this order and inject the preview
                        inspector script into the first one found:
                        <span className="block mt-1 font-mono">
                          {config.injectionStrategy?.config?.indexPath
                            ? `${config.injectionStrategy.config.indexPath} → `
                            : ''}
                          index.html → public/index.html → dist/index.html → build/index.html
                        </span>
                        <span className="block mt-1">
                          The script is injected only once into the {'<head>'} tag or at the
                          beginning of {'<body>'}.
                        </span>
                      </p>
                    ) : selectedInjectionMethod.value === 'middleware' ? (
                      <p className="text-xs text-muted-foreground">
                        Currently only implemented for Next.js applications. The system creates a
                        middleware file that adds headers to HTML responses, enabling script
                        injection without modifying source files.
                      </p>
                    ) : (
                      <p className="text-xs text-muted-foreground">
                        {selectedInjectionMethod.details}
                      </p>
                    )}
                  </div>
                )}
              </div>

              {config.injectionStrategy?.method === 'html-injection' && (
                <div className="space-y-2">
                  <Label htmlFor="indexPath">HTML File Path</Label>
                  <Input
                    id="indexPath"
                    placeholder="e.g., index.html or public/index.html"
                    value={config.injectionStrategy.config?.indexPath || ''}
                    onChange={(e) =>
                      setConfig({
                        ...config,
                        injectionStrategy: {
                          method: config.injectionStrategy?.method || 'html-injection',
                          config: {
                            ...(config.injectionStrategy?.config || {}),
                            indexPath: e.target.value,
                          },
                        },
                      })
                    }
                  />
                  <p className="text-xs text-muted-foreground">
                    Path to the main HTML file where scripts will be injected (relative to project
                    root)
                  </p>
                </div>
              )}
            </div>
          </div>

          <div className="flex justify-end">
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Updating...
                </>
              ) : (
                'Update Preview Config'
              )}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}

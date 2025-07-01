import { useState } from 'react'
import type { PreviewConfig, UpdateRepositoryRequest } from '@moatless/shared/types/repository'
import { 
  FRAMEWORK_OPTIONS, 
  INJECTION_METHOD_OPTIONS,
} from '@moatless/shared/constants/preview-config'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Badge } from '@/lib/components/ui/badge'
import { Collapsible, CollapsibleContent } from '@/lib/components/ui/collapsible'
import { Separator } from '@/lib/components/ui/separator'
import { Loader2, Wand2, ChevronDown, Info, CheckCircle2, XCircle } from 'lucide-react'

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
    serverPort: previewConfig?.serverPort || 3000,
    injectionStrategy: previewConfig?.injectionStrategy || {
      method: 'html-injection',
      config: {},
    },
  })

  const selectedInjectionMethod = INJECTION_METHOD_OPTIONS.find(
    option => option.value === (config.injectionStrategy?.method || 'html-injection')
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
          autoDetected: true,
        })
      }
    } finally {
      setIsDetecting(false)
    }
  }

  const handleFrameworkChange = (value: string) => {
    const framework = value as PreviewConfig['framework']
    
    setConfig({ 
      ...config, 
      framework,
      // Keep the current port - system uses random ports anyway
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
          {config.autoDetected && (
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
                <Select
                  value={config.framework}
                  onValueChange={handleFrameworkChange}
                >
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
                Command to start the development server. Leave empty to use the default for your framework.
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="serverPort">Server Port</Label>
              <Input
                id="serverPort"
                type="number"
                placeholder="3000"
                value={config.serverPort}
                onChange={(e) => setConfig({ ...config, serverPort: parseInt(e.target.value) || 3000 })}
              />
              <p className="text-xs text-muted-foreground">
                Used to replace {'{PORT}'} placeholders in server commands. The system always finds and uses random available ports for actual binding.
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
                  <ChevronDown className={`ml-2 h-4 w-4 transition-transform ${showInjectionDetails ? 'rotate-180' : ''}`} />
                </Button>
              </div>
              
              <Collapsible open={showInjectionDetails} onOpenChange={setShowInjectionDetails}>
                <CollapsibleContent className="space-y-4">
                  <div className="rounded-lg border p-4 bg-muted/50">
                    <h5 className="font-medium mb-2">What is Script Injection?</h5>
                    <p className="text-sm text-muted-foreground mb-3">
                      Script injection determines how preview inspector scripts are added to your application. 
                      Different methods work better with different frameworks and deployment setups.
                    </p>
                    
                    <div className="space-y-3">
                      {INJECTION_METHOD_OPTIONS.map((method) => (
                        <div key={method.value} className="border rounded p-3">
                          <div className="flex items-start justify-between mb-2">
                            <h6 className="font-medium text-sm">{method.label}</h6>
                            <Badge variant="outline" className="text-xs">
                              {method.value}
                            </Badge>
                          </div>
                          <p className="text-xs text-muted-foreground mb-2">{method.description}</p>
                          <p className="text-xs mb-2">{method.details}</p>
                          
                          <div className="grid grid-cols-1 md:grid-cols-2 gap-2 text-xs">
                            <div>
                              <div className="flex items-center gap-1 mb-1">
                                <CheckCircle2 className="h-3 w-3 text-green-500" />
                                <span className="font-medium">Pros:</span>
                              </div>
                              <ul className="list-disc list-inside text-muted-foreground ml-4 space-y-0.5">
                                {method.pros.map((pro, i) => <li key={i}>{pro}</li>)}
                              </ul>
                            </div>
                            <div>
                              <div className="flex items-center gap-1 mb-1">
                                <XCircle className="h-3 w-3 text-red-500" />
                                <span className="font-medium">Cons:</span>
                              </div>
                              <ul className="list-disc list-inside text-muted-foreground ml-4 space-y-0.5">
                                {method.cons.map((con, i) => <li key={i}>{con}</li>)}
                              </ul>
                            </div>
                          </div>
                          
                          <div className="mt-2 pt-2 border-t">
                            <span className="text-xs font-medium">Requirements: </span>
                            <span className="text-xs text-muted-foreground">{method.requirements}</span>
                          </div>
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
                    {INJECTION_METHOD_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        <div className="flex flex-col">
                          <span>{option.label}</span>
                          <span className="text-xs text-muted-foreground">{option.description}</span>
                        </div>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {selectedInjectionMethod && (
                  <div className="rounded border p-3 bg-muted/30">
                    <p className="text-sm mb-2">{selectedInjectionMethod.details}</p>
                    <p className="text-xs text-muted-foreground">
                      <strong>Requirements:</strong> {selectedInjectionMethod.requirements}
                    </p>
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
                    Path to the main HTML file where scripts will be injected (relative to project root)
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
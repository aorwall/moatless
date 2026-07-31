import { Check, Copy, Loader2, Smartphone } from 'lucide-react'
import { QRCodeSVG } from 'qrcode.react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import {
  type PairingCodeResponse,
  usePairingCode,
} from '@/features/settings/hooks/use-pairing-code'
import { ConfigurationGroup, ConfigurationSectionWrapper } from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { getErrorMessage } from '@/lib/utils/error'

/**
 * The one string the T3 Code app's scanner understands.
 *
 * It reads a scan as a pairing URL and splits it back into host and code
 * (`apps/mobile/src/features/connection/pairing.ts`), discarding the path — so
 * the origin and the token in the fragment are the whole payload, and the code
 * rides in the fragment rather than the query because a fragment is not sent to
 * the server or written to its logs.
 *
 * Exported because that parser lives in another codebase: this is the one thing
 * here a test can pin down without a phone.
 */
export function pairingUrl(code: string) {
  const url = new URL(window.location.origin)
  url.hash = new URLSearchParams([['token', code]]).toString()
  return url.toString()
}

/** `ABCDEFGHJKLM` reads back as `ABCDEF-GHJKLM`. The server ignores the dash. */
function grouped(code: string) {
  return `${code.slice(0, 6)}-${code.slice(6)}`
}

function secondsUntil(expiresAt: string) {
  return Math.max(0, Math.floor((new Date(expiresAt).getTime() - Date.now()) / 1000))
}

/**
 * How long the code on screen is still worth typing.
 *
 * Shown rather than left implicit because the window is minutes: without it a
 * code that has quietly died looks exactly like one that was mistyped.
 */
function Countdown({ expiresAt }: { expiresAt: string }) {
  const [remaining, setRemaining] = useState(() => secondsUntil(expiresAt))

  useEffect(() => {
    setRemaining(secondsUntil(expiresAt))
    const timer = setInterval(() => setRemaining(secondsUntil(expiresAt)), 1000)
    return () => clearInterval(timer)
  }, [expiresAt])

  if (remaining === 0) {
    return <span className="text-destructive">Expired — generate a new code</span>
  }
  const minutes = Math.floor(remaining / 60)
  const seconds = remaining % 60
  return <span>Expires in {`${minutes}:${seconds.toString().padStart(2, '0')}`}</span>
}

/** A read-only value with a button that copies it. */
function CopyableField({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false)

  const copy = () => {
    navigator.clipboard.writeText(value).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), 2000)
      },
      () => toast.error('Failed to copy to clipboard'),
    )
  }

  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <div className="flex items-center gap-2">
        <code className="flex-1 p-2 bg-muted rounded font-mono text-sm break-all">{value}</code>
        <Button variant="outline" size="sm" onClick={copy} className="shrink-0">
          {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
        </Button>
      </div>
    </div>
  )
}

/**
 * The code, once it exists: scannable first, typeable second.
 *
 * Both are the same code. The QR is what the app's scanner is for, and the
 * fields below it are what a device with no camera pointed at this screen —
 * or a person who would rather type — needs.
 */
function IssuedCode({ code }: { code: PairingCodeResponse }) {
  return (
    <div className="space-y-4 p-4 border rounded-lg bg-card">
      <div className="flex flex-col items-center gap-3">
        {/* White behind the QR at every theme: a scanner needs the contrast,
            and a dark surface under dark modules is not readable. */}
        <div className="bg-white p-3 rounded-lg">
          <QRCodeSVG value={pairingUrl(code.code)} size={176} marginSize={0} level="M" />
        </div>
        <p className="text-sm text-muted-foreground text-center">
          Scan this in T3 Code — Add Environment, then Scan QR Code.
        </p>
      </div>

      <div className="space-y-4 pt-2 border-t">
        <p className="text-sm text-muted-foreground">Or enter these by hand:</p>
        <CopyableField label="Server address" value={window.location.origin} />
        <CopyableField label="Pairing code" value={grouped(code.code)} />
      </div>

      <p className="text-sm text-muted-foreground">
        <Countdown expiresAt={code.expiresAt} />. The code is shown only here — generate another if
        you lose it.
      </p>
    </div>
  )
}

export function DevicePairingSection() {
  const createPairingCode = usePairingCode()
  const [label, setLabel] = useState('')
  const [code, setCode] = useState<PairingCodeResponse | null>(null)

  const handleGenerate = () => {
    createPairingCode.mutate(
      { label: label.trim() || null },
      {
        onSuccess: (result) => setCode(result),
        onError: (error) =>
          toast.error('Failed to create a pairing code', {
            description: getErrorMessage(error),
          }),
      },
    )
  }

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Pair a device" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect the T3 Code app on your phone or tablet to this account. Generate a code, then
          scan it in the app or enter this server's address and the code by hand. The code can be
          used once and is valid for a few minutes; the device keeps its own key afterwards, which
          appears under API Keys and is revoked there.
        </p>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="deviceLabel">Device name (optional)</Label>
            <Input
              id="deviceLabel"
              placeholder="e.g., My phone, Work iPad"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              What the paired device is called in the API Keys list.
            </p>
          </div>

          <Button onClick={handleGenerate} disabled={createPairingCode.isPending}>
            {createPairingCode.isPending ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <Smartphone className="mr-2 h-4 w-4" />
            )}
            {code ? 'Generate a new code' : 'Generate pairing code'}
          </Button>

          {code && <IssuedCode code={code} />}
        </div>
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}

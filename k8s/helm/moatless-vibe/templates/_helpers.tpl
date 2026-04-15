{{/*
Expand the name of the chart.
*/}}
{{- define "moatless-vibe.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "moatless-vibe.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "moatless-vibe.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "moatless-vibe.labels" -}}
helm.sh/chart: {{ include "moatless-vibe.chart" . }}
{{ include "moatless-vibe.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "moatless-vibe.selectorLabels" -}}
app.kubernetes.io/name: {{ include "moatless-vibe.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "moatless-vibe.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "moatless-vibe.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Get the secrets name
*/}}
{{- define "moatless-vibe.secretsName" -}}
{{- if .Values.externalSecrets.enabled }}
{{- .Values.externalSecrets.secretName }}
{{- else }}
{{- include "moatless-vibe.fullname" . }}-secrets
{{- end }}
{{- end }}

{{/*
Get the sandbox image
Priority: kubernetes.sandboxImage (object or string) > sandboxImage section > fallback to main image
Supports both:
  --set kubernetes.sandboxImage.repository=... --set kubernetes.sandboxImage.tag=...
  --set kubernetes.sandboxImage=full/image:tag
*/}}
{{- define "moatless-vibe.sandboxImage" -}}
{{- if .Values.kubernetes.sandboxImage.repository }}
{{- /* Object format: kubernetes.sandboxImage.repository + tag */}}
{{- $tag := default (default .Chart.AppVersion .Values.image.tag) .Values.kubernetes.sandboxImage.tag }}
{{- printf "%s:%s" .Values.kubernetes.sandboxImage.repository $tag }}
{{- else if and .Values.kubernetes.sandboxImage (kindIs "string" .Values.kubernetes.sandboxImage) }}
{{- /* String format: kubernetes.sandboxImage is a full image string */}}
{{- .Values.kubernetes.sandboxImage }}
{{- else if .Values.sandboxImage.repository }}
{{- /* Fallback to sandboxImage section */}}
{{- $tag := default .Values.image.tag .Values.sandboxImage.tag }}
{{- printf "%s:%s" .Values.sandboxImage.repository (default .Chart.AppVersion $tag) }}
{{- else }}
{{- /* Final fallback: main image */}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) }}
{{- end }}
{{- end }}

{{/*
Get the Kubernetes namespace for sandbox pods
*/}}
{{- define "moatless-vibe.sandboxNamespace" -}}
{{- if .Values.kubernetes.namespace }}
{{- .Values.kubernetes.namespace }}
{{- else }}
{{- .Release.Namespace }}
{{- end }}
{{- end }}

{{/*
Get the Redis URL
Priority: redis.url (explicit) > auto-generated from subchart (if redis.enabled=true)
*/}}
{{- define "moatless-vibe.redisUrl" -}}
{{- if .Values.redis.url }}
{{- .Values.redis.url }}
{{- else if .Values.redis.enabled }}
{{- printf "redis://%s-redis-master:6379" .Release.Name }}
{{- else }}
{{- fail "Redis configuration required. Either:\n  - set redis.url to point to an external Redis (e.g. AWS ElastiCache), or\n  - set redis.enabled=true to deploy the bundled Bitnami Redis subchart.\nSee k8s/helm/moatless-vibe/README.md for details." }}
{{- end }}
{{- end }}

{{/*
Guard: the two ingress modes (standard Ingress and Traefik IngressRoute) are
mutually exclusive. Enabling both causes duplicate / conflicting routes.
*/}}
{{- define "moatless-vibe.validateIngressMode" -}}
{{- if and .Values.ingress.enabled .Values.proxy.traefikIngressRoute.enabled -}}
{{- fail "Conflicting ingress modes: both ingress.enabled=true and proxy.traefikIngressRoute.enabled=true are set. Pick one — set ingress.enabled=false if you use Traefik IngressRoute CRDs, or proxy.traefikIngressRoute.enabled=false otherwise." -}}
{{- end -}}
{{- end -}}

{{/*
Sandbox external egress CIDR allowlists per category.
Used by sandbox-networkpolicy.yaml when networkPolicy.sandbox.broadEgress=false.

NOTE: FQDN-based egress is not expressible in stock Kubernetes NetworkPolicy.
The CIDR lists below are best-effort snapshots of published IP ranges for each
provider. They will drift over time and may produce false-positive DROPs when
upstream CIDRs change. For production-grade egress filtering use Cilium
(networkPolicy.sandbox.cilium.enabled=true) with FQDN matching.

To customize, override networkPolicy.sandbox.extraAllowedCidrs in values.yaml.
*/}}
{{- define "moatless-vibe.sandboxEgressCidrs" -}}
cidrs:
  anthropic:
    # api.anthropic.com is served via Cloudflare. Narrower than 0.0.0.0/0 but
    # still covers all Cloudflare-fronted services. Replace with your gateway
    # IP if you use a private network egress (e.g. AWS PrivateLink).
    - 104.16.0.0/13
    - 104.24.0.0/14
    - 172.64.0.0/13
    - 162.159.0.0/16
  github:
    # GitHub meta (api.github.com, github.com, codeload.github.com)
    # Source: https://api.github.com/meta — api + web + git ranges.
    - 140.82.112.0/20
    - 143.55.64.0/20
    - 192.30.252.0/22
    - 185.199.108.0/22
    - 20.200.245.247/32
    - 20.205.243.166/32
    - 20.233.83.145/32
  npm:
    # registry.npmjs.org is served via Cloudflare.
    - 104.16.0.0/13
    - 104.24.0.0/14
    - 172.64.0.0/13
    - 162.159.0.0/16
  pypi:
    # pypi.org + files.pythonhosted.org served via Fastly.
    - 151.101.0.0/16
    - 199.232.0.0/16
  cratesIo:
    # crates.io + static.crates.io (CloudFront) and index.crates.io (Fastly).
    - 151.101.0.0/16
    - 13.32.0.0/15
    - 13.224.0.0/14
    - 52.84.0.0/15
    - 54.192.0.0/16
    - 54.230.0.0/16
  goModules:
    # proxy.golang.org + sum.golang.org (Google frontend).
    - 142.250.0.0/15
    - 172.217.0.0/16
    - 216.58.192.0/19
    - 74.125.0.0/16
  ubuntuApt:
    # archive.ubuntu.com + security.ubuntu.com — Canonical's global CDN mirrors.
    # These change more frequently than the others; prefer Cilium FQDN when
    # enabling this category in regulated environments.
    - 91.189.88.0/21
    - 185.125.190.0/24
  playwrightCdn:
    # playwright.azureedge.net (Azure Front Door) — only needed when sandboxes
    # run `npx playwright install` at runtime. Browsers are typically baked
    # into the sandbox image at build time, in which case you can leave this off.
    - 13.107.246.0/24
    - 13.107.213.0/24
    - 150.171.8.0/24
{{- end -}}

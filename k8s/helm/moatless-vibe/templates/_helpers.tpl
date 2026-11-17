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
Name of the Service that answers the SPA catch-all on the app host.

T3 and the Moatless SPA cannot share an origin — both own `/` and every path
under it that is not a backend route. With t3.host set they get one origin
each, so the app host keeps the Moatless SPA; without it, T3 takes the app host
over. Both routing modes read this, so the two cannot disagree about which
front end is being served.
*/}}
{{- define "moatless-vibe.spaServiceName" -}}
{{- if (and .Values.t3.enabled (not .Values.t3.host)) }}
{{- printf "%s-t3" (include "moatless-vibe.fullname" .) }}
{{- else }}
{{- include "moatless-vibe.fullname" . }}
{{- end }}
{{- end }}

{{/*
Hostname the T3 UI is served on: its own when t3.host is set, the app host
otherwise. Empty when T3 is not deployed.

Setting t3.host TO the app host is rejected rather than treated as either
meaning. It is the one value that makes the two modes contradict each other:
t3.host set says "give T3 its own origin", so the app host keeps the Moatless
SPA and a second catch-all is rendered for the T3 host — which is now the same
host, at the same priority, pointing somewhere else. Traefik gets two routers
with an identical rule; ALB gets two `/` rules at group.order 100. Which front
end answers `/` then depends on the controller, not the chart.

The two things it could have meant are both already reachable: leave t3.host
empty to serve T3 on the app host in place of the Moatless SPA, or set it to a
different hostname to serve both.
*/}}
{{- define "moatless-vibe.t3Host" -}}
{{- if .Values.t3.enabled -}}
{{- $appHost := (index .Values.ingress.hosts 0).host -}}
{{- if (and .Values.t3.host (eq .Values.t3.host $appHost)) -}}
{{- fail (printf "t3.host is set to %q, which is already the app host (ingress.hosts[0].host). Leave t3.host empty to serve the T3 UI on the app host in place of the Moatless SPA, or set it to a different hostname to serve both." .Values.t3.host) -}}
{{- end -}}
{{- .Values.t3.host | default $appHost -}}
{{- end -}}
{{- end }}

{{/*
Every hostname the backend has to answer its own routes on. The app host
always, plus the T3 host when that is a separate one — a T3 client served from
its own origin calls /api, /ws, /.well-known/t3 and /oauth/token relative to
that origin, so the edge has to route them there too.
*/}}
{{- define "moatless-vibe.backendHosts" -}}
{{- $hosts := list (index .Values.ingress.hosts 0).host -}}
{{- if (and .Values.t3.enabled .Values.t3.host) -}}
{{- $hosts = append $hosts .Values.t3.host -}}
{{- end -}}
{{- toJson $hosts -}}
{{- end }}

{{/*
Browser origin the T3 UI runs on, e.g. https://vibe-t3.example.com.

This is the Origin header the backend sees on its WebSocket upgrade, so it is
what T3_UI_RPC_ALLOWED_ORIGINS has to contain. Derived rather than configured:
a second place to write the host is a second place for it to be wrong, and a
mismatch fails at the socket upgrade — after the page has loaded and looked
fine.
*/}}
{{- define "moatless-vibe.t3Origin" -}}
{{- printf "%s://%s" (.Values.proxy.protocol | default "https") (include "moatless-vibe.t3Host" .) -}}
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
Get the DATABASE_URL for the bundled first-party Postgres (postgres.enabled=true).
Resolves to postgres://{user}:{pass}@{service}:5432/{db} against the in-cluster Service.
*/}}
{{- define "moatless-vibe.databaseUrl" -}}
{{- printf "postgres://%s:%s@%s:5432/%s" .Values.postgres.auth.username .Values.postgres.auth.password .Values.postgres.service.name .Values.postgres.auth.database -}}
{{- end }}

{{/*
No-op stub — preserved so existing `include "moatless-vibe.validateIngressMode"`
callers still render. The earlier guard rejected `ingress.enabled=true` AND
`proxy.traefikIngressRoute.enabled=true` together, but the two modes target
different concerns: `ingress` (typically ALB) for the main app host, and the
Traefik IngressRoute CRD for cluster-internal routing of preview/sandbox
traffic. They coexist without conflict in production.
*/}}
{{- define "moatless-vibe.validateIngressMode" -}}
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

{{/*
Rule set for the `cluster-read` sandbox-access preset.

Chart-owned rather than values-owned on purpose: this is the list that decides
what an opted-in sandbox can see, so it belongs where code review looks. Keeping
it here also means a chart upgrade can widen it (a new apiGroup, a new
subresource) without every install having pinned its own stale copy in gitops.

get/list/watch only, and `secrets` is deliberately absent — a sandbox runs an
LLM agent with internet egress, so cluster-wide Secret read would be a
credential-exfiltration path. Use the `namespace-admin` preset when Secret
access is genuinely required; it is scoped to two namespaces instead.
*/}}
{{- define "moatless-vibe.clusterReadRules" -}}
- apiGroups: [""]
  resources:
    - namespaces
    - nodes
    - pods
    - pods/log
    - services
    - endpoints
    - configmaps
    - serviceaccounts
    - persistentvolumes
    - persistentvolumeclaims
    - replicationcontrollers
    - limitranges
    - resourcequotas
    - events
  verbs: ["get", "list", "watch"]
- apiGroups:
    - apps
    - batch
    - autoscaling
    - networking.k8s.io      # NetworkPolicy, Ingress
    - policy                 # PodDisruptionBudget
    - storage.k8s.io
    - discovery.k8s.io
    - coordination.k8s.io
    - node.k8s.io
    - scheduling.k8s.io
    - rbac.authorization.k8s.io
    - admissionregistration.k8s.io
    - apiextensions.k8s.io   # CustomResourceDefinition
    - apiregistration.k8s.io
    - metrics.k8s.io
  resources: ["*"]
  verbs: ["get", "list", "watch"]
# Non-resource endpoints — `kubectl version`, `/healthz`, discovery.
- nonResourceURLs: ["/healthz", "/livez", "/readyz", "/version", "/metrics", "/openapi/*"]
  verbs: ["get"]
{{- end -}}

{{/*
Validate one sandbox-access profile and return its `access` preset.
Usage: {{ include "moatless-vibe.accessPreset" (dict "name" $name "profile" $p) }}
*/}}
{{- define "moatless-vibe.accessPreset" -}}
{{- $known := list "cluster-read" "namespace-admin" -}}
{{- if not (has .profile.access $known) -}}
{{- fail (printf "sandboxAccess.profiles.%s.access=%q is not a known preset — use one of %v. An unrecognised value would create a ServiceAccount with no RBAC at all, which fails at kubectl time rather than at deploy time." .name (.profile.access | toString) $known) -}}
{{- end -}}
{{- if not .profile.serviceAccount -}}
{{- fail (printf "sandboxAccess.profiles.%s.serviceAccount is empty — it names the ServiceAccount the RBAC binds to and the value a workspace puts in sandboxServiceAccount, so an empty one would bind nothing." .name) -}}
{{- end -}}
{{- .profile.access -}}
{{- end -}}

{{/*
Comma-joined network profiles a chart template selects, beyond
egressProxy.profiles: every sandboxAccess profile key (enabled or not, so
disabling a profile does not fail a write that names it), every
networkPolicy.sandbox.profileEgress key, and kubernetes.extraNetworkProfiles
for policies managed outside the chart.
*/}}
{{- define "moatless-vibe.sandboxNetworkProfiles" -}}
{{- $names := list -}}
{{- range $name, $_ := .Values.sandboxAccess.profiles }}{{ $names = append $names $name }}{{ end -}}
{{- range $name, $_ := .Values.networkPolicy.sandbox.profileEgress }}{{ $names = append $names $name }}{{ end -}}
{{- range .Values.kubernetes.extraNetworkProfiles }}{{ $names = append $names . }}{{ end -}}
{{- $names | uniq | join "," -}}
{{- end -}}

{{/*
Comma-joined service accounts a workspace may run its sandboxes as, or empty
(no restriction) while kubernetes.allowedSandboxServiceAccounts is empty. Once
set, the ServiceAccounts the chart creates for enabled sandboxAccess profiles
and shadowDev join it. K8S_SANDBOX_SERVICE_ACCOUNT deliberately does not: a
workspace naming it mounts its token, and by default it is the backend SA.
*/}}
{{- define "moatless-vibe.allowedSandboxServiceAccounts" -}}
{{- with .Values.kubernetes.allowedSandboxServiceAccounts -}}
{{- $names := . -}}
{{- range $_, $profile := $.Values.sandboxAccess.profiles }}
{{- if $profile.enabled }}{{ $names = append $names $profile.serviceAccount }}{{ end }}
{{- end -}}
{{- if $.Values.shadowDev.enabled }}{{ $names = append $names $.Values.shadowDev.serviceAccount }}{{ end -}}
{{- $names | uniq | join "," -}}
{{- end -}}
{{- end -}}

{{/*
SANDBOX_ROLE_IDENTITIES_JSON from sandboxAccess.roleDefaults, or empty when no
role names a profile. Fails on a role naming a profile that is not enabled:
the backend would mount a ServiceAccount that does not exist.
*/}}
{{- define "moatless-vibe.sandboxRoleIdentities" -}}
{{- $out := dict -}}
{{- $defaults := .Values.sandboxAccess.roleDefaults | default dict -}}
{{- range $role := list "admin" "user" }}
{{- with index $defaults $role }}
{{- $profile := index $.Values.sandboxAccess.profiles . | default dict }}
{{- if not $profile.enabled }}
{{- fail (printf "sandboxAccess.roleDefaults.%s names %q, which is not an enabled sandboxAccess profile" $role .) }}
{{- end }}
{{- $_ := set $out $role (dict "serviceAccount" $profile.serviceAccount "networkProfile" .) }}
{{- end }}
{{- end -}}
{{- if $out }}{{ $out | toJson }}{{ end -}}
{{- end -}}

{{/*
Ingress rules for a sandbox pod: backend (gRPC + health), Traefik preview, and
extraIngress. Shared by the base and the sealed-egress policies, rendered at
the `ingress:` list indent.
*/}}
{{- define "moatless-vibe.sandboxIngressRules" -}}
# Backend → sandbox gRPC (control plane) and sandbox HTTP health
- from:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ .Release.Namespace }}
      podSelector:
        matchLabels:
          app.kubernetes.io/name: {{ include "moatless-vibe.name" . }}
          app.kubernetes.io/component: backend
  ports:
    - protocol: TCP
      port: {{ .Values.kubernetes.sandboxGrpcPort }}
    - protocol: TCP
      port: 8001
{{- with .Values.networkPolicy.sandbox.ingressFromTraefik }}
# Traefik → sandbox dev servers (preview proxy routes to user dev servers)
- from:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ .namespace }}
      {{- if .podLabels }}
      podSelector:
        matchLabels:
          {{- toYaml .podLabels | nindent 10 }}
      {{- end }}
  ports:
    {{- range .ports }}
    - protocol: TCP
      port: {{ . }}
    {{- end }}
{{- end }}
{{- with .Values.networkPolicy.sandbox.extraIngress }}
{{ toYaml . }}
{{- end }}
{{- end }}

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
{{- fail "Redis configuration required: either set redis.url or enable redis.enabled=true" }}
{{- end }}
{{- end }}

{{- define "maidan.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "maidan.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "maidan.labels" -}}
app.kubernetes.io/name: {{ include "maidan.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: server
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
{{- end }}

{{- define "maidan.selectorLabels" -}}
app.kubernetes.io/name: {{ include "maidan.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: server
{{- end }}

{{/*
The server image. A digest, when set, is the reference — immutable, so a
re-pointed tag cannot change what runs. The tag is then informational only.
*/}}
{{- define "maidan.image" -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository .Values.image.tag -}}
{{- end -}}
{{- end -}}

{{/*
Name of the Secret holding runtime secrets (DATABASE_URL, …). When
`.Values.existingSecret` is set the chart references that pre-created Secret and
renders none of its own; otherwise it uses the chart-managed Secret.
*/}}
{{- define "maidan.secretName" -}}
{{- if .Values.existingSecret }}
{{- .Values.existingSecret }}
{{- else }}
{{- printf "%s-secrets" (include "maidan.fullname" .) }}
{{- end }}
{{- end }}

{{/*
Refusals checked before anything renders. `production` is set by the prod
values files and left off by dev and CI, which still render the local
`maidan-server:dev` image. A production install otherwise inherits that image
from values.yaml without a word, which is how the stack's prod values shipped
`maidan-server:dev`.
*/}}
{{- define "maidan.validate" -}}
{{- if .Values.production }}
{{- $repo := required "image.repository is required for a production install: set it to ghcr.io/david-engelmann/maidan-server (or your mirror of it)" .Values.image.repository }}
{{- if eq $repo "maidan-server" }}
{{- fail "image.repository is maidan-server, the local development image: set it to ghcr.io/david-engelmann/maidan-server (or your mirror of it)" }}
{{- end }}
{{- if not .Values.image.digest }}
{{- $tag := required "a production install needs an explicit image: set image.tag to a release (vN.0.0) or image.digest to its sha256" .Values.image.tag }}
{{- if has $tag (list "dev" "latest") }}
{{- fail (printf "image.tag %q is not a release: set image.tag to a release (vN.0.0) or image.digest to its sha256" $tag) }}
{{- end }}
{{- end }}
{{- end }}
{{- end -}}

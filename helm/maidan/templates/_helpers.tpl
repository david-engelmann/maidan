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
`maidan-server:dev` image and the development DATABASE_URL. A production
install otherwise inherits both from values.yaml without a word, which is how
the stack's prod values shipped `maidan-server:dev`. A `CHANGE_ME` placeholder
is refused in every render: it is never a working value.
*/}}
{{- define "maidan.validate" -}}
{{- range $k, $v := .Values.config }}
{{- if contains "CHANGE_ME" (toString $v) }}
{{- fail (printf "config.%s still holds the placeholder CHANGE_ME: set it to the real value" $k) }}
{{- end }}
{{- end }}
{{- if not .Values.existingSecret }}
{{- range $k, $v := .Values.secrets }}
{{- if contains "CHANGE_ME" (toString $v) }}
{{- fail (printf "secrets.%s still holds the placeholder CHANGE_ME: set it to the real value, or set existingSecret to a Secret that holds %s" $k $k) }}
{{- end }}
{{- end }}
{{- range $k := list "contentKek" "contentKekPrevious" }}
{{- if contains "CHANGE_ME" (toString (index $.Values $k)) }}
{{- fail (printf "%s still holds the placeholder CHANGE_ME: set it to a key from openssl rand -hex 32, or set existingSecret to a Secret holding MAIDAN_CONTENT_KEK" $k) }}
{{- end }}
{{- end }}
{{- if .Values.production }}
{{- $db := required "a production install needs its database: set existingSecret to a Secret holding DATABASE_URL and MAIDAN_CONTENT_KEK, or set secrets.DATABASE_URL" .Values.secrets.DATABASE_URL }}
{{- if eq $db "postgres://maidan:maidan@postgres:5432/maidan" }}
{{- fail "secrets.DATABASE_URL is the chart's development default (user and password maidan): set existingSecret to a Secret holding DATABASE_URL and MAIDAN_CONTENT_KEK, or set secrets.DATABASE_URL to your database" }}
{{- end }}
{{- range $k, $v := .Values.secrets }}
{{- if not $v }}
{{- fail (printf "secrets.%s is empty: set it, or set existingSecret to a Secret that holds %s" $k $k) }}
{{- end }}
{{- end }}
{{- end }}
{{- end }}
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

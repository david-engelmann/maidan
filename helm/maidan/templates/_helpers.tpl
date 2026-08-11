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

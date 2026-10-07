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
{{- if contains "CHANGE_ME" (toString .Values.image.digest) }}
{{- fail "image.digest still holds the placeholder CHANGE_ME: set it to a real sha256 digest" }}
{{- end }}
{{- if contains "CHANGE_ME" (toString .Values.image.tag) }}
{{- fail "image.tag still holds the placeholder CHANGE_ME: set it to a release tag" }}
{{- end }}
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
in `config`, and in `secrets` or a content KEK when no `existingSecret` is set,
is refused: it is never a working value. `image.tag` and `image.digest` holding
one are refused by `maidan.image`, including when `existingSecret` is set.
`extraEnvFrom` is read after `secrets` and may supply DATABASE_URL, which the
render cannot see, so the DATABASE_URL checks are left to whoever sets it (the
stack checks it when it runs no database of its own).
*/}}
{{/*
The secrets the server can read from a file, as maidan-env's SECRET_FILE_ENV
lists them. `every_secret_file_the_chart_mounts_is_one_the_server_reads` in
crates/maidan-env keeps the two lists equal.
*/}}
{{- define "maidan.fileSecretNames" -}}
DATABASE_URL FEDERATION_DECRYPT_KEYS FEDERATION_ENCRYPTION_KEY MAIDAN_CONTENT_KEK MAIDAN_CONTENT_KEK_PREVIOUS MAIDAN_DB_REPLICA_URL MAIDAN_EMBEDDING_API_KEY MAIDAN_EXPORT_SIGNING_KEY MAIDAN_GITHUB_TOKEN MAIDAN_GITHUB_WEBHOOK_SECRET MAIDAN_OIDC_CLIENT_SECRET MAIDAN_RATE_LIMIT_REDIS_URL MAIDAN_SESSION_SECRET MAIDAN_SLACK_BOT_TOKEN MAIDAN_SLACK_SIGNING_SECRET MAIDAN_SMTP_PASSWORD MAIDAN_SUBSCRIBE_RESUME_SECRET MAIDAN_VAPID_PRIVATE_KEY S3_ACCESS_KEY_ID S3_SECRET_ACCESS_KEY
{{- end -}}

{{/*
The Secrets mounted as files, as a JSON list of {name, keys}: each
`secretFiles.extra` source, then the chart's own Secret with every key it
renders, or existingSecret with the keys declared for it. A key an extra source
holds comes from that source and is left out of the chart's own Secret, the way
extraEnvFrom overrides it. Refuses a key the server cannot read from a file, and
a key two extra sources both hold.
*/}}
{{- define "maidan.secretFileSources" -}}
{{- $sources := list }}
{{- $taken := dict }}
{{- range .Values.secretFiles.extra }}
{{- $name := tpl .name $ }}
{{- if not .keys }}
{{- fail (printf "secretFiles.extra lists Secret %s with no keys: name each key to mount, since a source with none would mount every key it holds" $name) }}
{{- end }}
{{- range .keys }}
{{- if hasKey $taken . }}
{{- fail (printf "%s is mounted from both %s and %s: keep it in one" . (index $taken .) $name) }}
{{- end }}
{{- $_ := set $taken . $name }}
{{- end }}
{{- $sources = append $sources (dict "name" $name "keys" .keys) }}
{{- end }}
{{- $own := list }}
{{- if .Values.existingSecret }}
{{- $own = .Values.secretFiles.existingSecretKeys }}
{{- if not $own }}
{{- fail "secretFiles.existingSecretKeys is empty: name each key existingSecret holds, since a source with none would mount every key it holds" }}
{{- end }}
{{- else }}
{{- $own = keys .Values.secrets | sortAlpha }}
{{- $own = append $own "MAIDAN_CONTENT_KEK" }}
{{- if .Values.contentKekPrevious }}
{{- $own = append $own "MAIDAN_CONTENT_KEK_PREVIOUS" }}
{{- end }}
{{- end }}
{{- $kept := list }}
{{- range $own }}
{{- if not (hasKey $taken .) }}
{{- $kept = append $kept . }}
{{- end }}
{{- end }}
{{- if $kept }}
{{- $sources = append $sources (dict "name" (include "maidan.secretName" .) "keys" $kept) }}
{{- end }}
{{- $allowed := splitList " " (include "maidan.fileSecretNames" .) }}
{{- range $sources }}
{{- $source := .name }}
{{- range .keys }}
{{- if not (has . $allowed) }}
{{- fail (printf "%s in Secret %s cannot be read from a file: put it in config, or set secretFiles.enabled=false" . $source) }}
{{- end }}
{{- end }}
{{- end }}
{{- toJson $sources }}
{{- end -}}

{{- define "maidan.validate" -}}
{{- if .Values.secretFiles.enabled }}
{{- range .Values.extraEnvFrom }}
{{- if .secretRef }}
{{- fail (printf "extraEnvFrom holds a secretRef to %s, which would put its keys in the environment: list it in secretFiles.extra with its keys, or set secretFiles.enabled=false" (tpl (toString .secretRef.name) $)) }}
{{- end }}
{{- end }}
{{- end }}
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
{{- $dbFromExtra := false }}
{{- if .Values.secretFiles.enabled }}
{{- range .Values.secretFiles.extra }}
{{- if has "DATABASE_URL" .keys }}
{{- $dbFromExtra = true }}
{{- end }}
{{- end }}
{{- end }}
{{- if .Values.production }}
{{- if not (or .Values.extraEnvFrom $dbFromExtra) }}
{{- $db := required "a production install needs its database: set existingSecret to a Secret holding DATABASE_URL and MAIDAN_CONTENT_KEK, or set secrets.DATABASE_URL" .Values.secrets.DATABASE_URL }}
{{- if eq $db "postgres://maidan:maidan@postgres:5432/maidan" }}
{{- fail "secrets.DATABASE_URL is the chart's development default (user and password maidan): set existingSecret to a Secret holding DATABASE_URL and MAIDAN_CONTENT_KEK, or set secrets.DATABASE_URL to your database" }}
{{- end }}
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

{{/*
Resource names. The Services keep the names the Bitnami subcharts gave them,
`<release>-postgresql` and `<release>-minio`, so connection strings written for
those still resolve.
*/}}
{{- define "maidan-stack.postgresql.fullname" -}}
{{- printf "%s-postgresql" .Release.Name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "maidan-stack.minio.fullname" -}}
{{- printf "%s-minio" .Release.Name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
The Secret that connects the server to the bundled stores. The server reads it
after its own ConfigMap and Secret (`maidan.extraEnvFrom`), so its keys win.
*/}}
{{- define "maidan-stack.datastores.secretName" -}}
{{- printf "%s-datastores" .Release.Name }}
{{- end }}

{{/* Labels; call with (dict "ctx" $ "name" "postgresql" "component" "database"). */}}
{{- define "maidan-stack.selectorLabels" -}}
app.kubernetes.io/name: {{ .name }}
app.kubernetes.io/instance: {{ .ctx.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end }}

{{- define "maidan-stack.labels" -}}
{{ include "maidan-stack.selectorLabels" . }}
app.kubernetes.io/part-of: maidan
app.kubernetes.io/managed-by: {{ .ctx.Release.Service }}
helm.sh/chart: {{ .ctx.Chart.Name }}-{{ .ctx.Chart.Version }}
{{- end }}

{{/*
An image reference; call with (dict "image" .Values.x.image "path" "x.image").
A digest, when set, is the reference, which a re-pointed tag cannot change.
*/}}
{{- define "maidan-stack.image" -}}
{{- $image := .image }}
{{- range $k := list "repository" "tag" "digest" }}
{{- if contains "CHANGE_ME" (toString (index $image $k)) }}
{{- fail (printf "%s.%s still holds the placeholder CHANGE_ME: set it to the real value" $.path $k) }}
{{- end }}
{{- end }}
{{- if not $image.repository }}
{{- fail (printf "%s.repository is empty: set it" .path) }}
{{- end }}
{{- if $image.digest -}}
{{- printf "%s@%s" $image.repository $image.digest -}}
{{- else if $image.tag -}}
{{- printf "%s:%s" $image.repository $image.tag -}}
{{- else -}}
{{- fail (printf "%s has neither a tag nor a digest: set %s.digest to a sha256 digest or %s.tag" .path .path .path) -}}
{{- end -}}
{{- end -}}

{{/*
A production render (`maidan.production`) names a release, not a moving tag:
a digest, or a tag other than dev, latest or empty. `dev` names the local
development repository, which a production render also refuses.
*/}}
{{- define "maidan-stack.releaseImage" -}}
{{- $image := .image }}
{{- if and .dev (eq (toString (default "" $image.repository)) .dev) }}
{{- fail (printf "%s.repository is %s, the local development image: set it to %s (or your mirror of it)" .path .dev .published) }}
{{- end }}
{{- if not $image.digest }}
{{- $tag := toString (default "" $image.tag) }}
{{- if has $tag (list "" "dev" "latest") }}
{{- fail (printf "%s.tag %q is not a release: a production install pins one, so set %s.digest to a sha256 digest or %s.tag to a release" .path $tag .path .path) }}
{{- end }}
{{- end }}
{{- end }}

{{/* The buckets `minio.defaultBuckets` names (comma-separated), as a list. */}}
{{- define "maidan-stack.minio.buckets" -}}
{{- $buckets := list }}
{{- range splitList "," (toString .Values.minio.defaultBuckets) }}
{{- if trim . }}
{{- $buckets = append $buckets (trim .) }}
{{- end }}
{{- end }}
{{- toJson $buckets }}
{{- end }}

{{/* userinfo and path escaping for DATABASE_URL: QueryEscape, with a space as %20. */}}
{{- define "maidan-stack.urlEscape" -}}
{{- urlquery . | replace "+" "%20" }}
{{- end }}

{{/*
Refusals checked before anything renders. A CHANGE_ME placeholder fails every
render; so do values the bundled stores cannot start with. A production render
(`maidan.production`, set by values-prod.yaml) also refuses development
credentials and an unpinned image. The server's DATABASE_URL comes from the
bundled database when it runs, so the maidan chart leaves that check to the
stack (it cannot see this chart's values); the stack makes it when
`postgresql.enabled` is off.
*/}}
{{- define "maidan-stack.validate" -}}
{{- $production := .Values.maidan.production }}
{{- $pg := .Values.postgresql }}
{{- $minio := .Values.minio }}
{{- range $k, $v := $pg.auth }}
{{- if contains "CHANGE_ME" (toString $v) }}
{{- fail (printf "postgresql.auth.%s still holds the placeholder CHANGE_ME: set it to the real value" $k) }}
{{- end }}
{{- end }}
{{- range $k, $v := $minio.auth }}
{{- if contains "CHANGE_ME" (toString $v) }}
{{- fail (printf "minio.auth.%s still holds the placeholder CHANGE_ME: set it to the real value" $k) }}
{{- end }}
{{- end }}
{{- if contains "CHANGE_ME" (toString $minio.defaultBuckets) }}
{{- fail "minio.defaultBuckets still holds the placeholder CHANGE_ME: set it to the bucket names" }}
{{- end }}
{{- if or $pg.enabled $minio.enabled }}
{{- $want := include "maidan-stack.datastores.secretName" . }}
{{- $wired := false }}
{{- range .Values.maidan.extraEnvFrom }}
{{- if and .secretRef (eq (tpl (toString .secretRef.name) $) $want) }}
{{- $wired = true }}
{{- end }}
{{- end }}
{{- if not $wired }}
{{- fail (printf "maidan.extraEnvFrom must keep the secretRef to %s (values.yaml has it): it is how the server reaches the bundled Postgres and MinIO" $want) }}
{{- end }}
{{- end }}
{{- if $pg.enabled }}
{{- range $k := list "username" "password" "database" }}
{{- if not (index $pg.auth $k) }}
{{- fail (printf "postgresql.auth.%s is empty: set it (for the password, openssl rand -hex 24)" $k) }}
{{- end }}
{{- end }}
{{- if $production }}
{{- if eq (toString $pg.auth.password) "maidan" }}
{{- fail "postgresql.auth.password is the chart's development default (maidan): set it to your own (openssl rand -hex 24)" }}
{{- end }}
{{- include "maidan-stack.releaseImage" (dict "image" $pg.image "path" "postgresql.image" "dev" "maidan-postgres" "published" "ghcr.io/david-engelmann/maidan-postgres") }}
{{- end }}
{{- else if and $production (not .Values.maidan.existingSecret) }}
{{- $db := required "a production install needs its database: enable postgresql, set maidan.existingSecret to a Secret holding DATABASE_URL and MAIDAN_CONTENT_KEK, or set maidan.secrets.DATABASE_URL" .Values.maidan.secrets.DATABASE_URL }}
{{- if eq $db "postgres://maidan:maidan@postgres:5432/maidan" }}
{{- fail "maidan.secrets.DATABASE_URL is the maidan chart's development default (user and password maidan): enable postgresql, set maidan.existingSecret to a Secret holding DATABASE_URL and MAIDAN_CONTENT_KEK, or set maidan.secrets.DATABASE_URL to your database" }}
{{- end }}
{{- end }}
{{- if $minio.enabled }}
{{- /* MinIO refuses to start below these lengths. */}}
{{- if lt (len (toString $minio.auth.rootUser)) 3 }}
{{- fail "minio.auth.rootUser is shorter than 3 characters, which MinIO refuses: set it" }}
{{- end }}
{{- if lt (len (toString $minio.auth.rootPassword)) 8 }}
{{- fail "minio.auth.rootPassword is shorter than 8 characters, which MinIO refuses: set it (openssl rand -hex 24)" }}
{{- end }}
{{- /* The bucket Job passes them in MC_HOST_local, where a colon ends a field. */}}
{{- range $k := list "rootUser" "rootPassword" }}
{{- if contains ":" (toString (index $minio.auth $k)) }}
{{- fail (printf "minio.auth.%s holds a colon, which MC_HOST_local cannot carry: choose one without (openssl rand -hex 24)" $k) }}
{{- end }}
{{- end }}
{{- if not (fromJsonArray (include "maidan-stack.minio.buckets" .)) }}
{{- fail "minio.defaultBuckets is empty: name at least one bucket (the server stores artifacts in the first)" }}
{{- end }}
{{- if $production }}
{{- if eq (toString $minio.auth.rootPassword) "minioadmin" }}
{{- fail "minio.auth.rootPassword is the chart's development default (minioadmin): set it to your own (openssl rand -hex 24)" }}
{{- end }}
{{- include "maidan-stack.releaseImage" (dict "image" $minio.image "path" "minio.image") }}
{{- include "maidan-stack.releaseImage" (dict "image" $minio.clientImage "path" "minio.clientImage") }}
{{- end }}
{{- end }}
{{- end -}}

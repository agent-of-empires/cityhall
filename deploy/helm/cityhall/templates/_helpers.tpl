{{- define "cityhall.name" -}}
{{- .Release.Name -}}
{{- end -}}

{{- define "cityhall.labels" -}}
app.kubernetes.io/name: cityhall
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "cityhall.selector" -}}
app.kubernetes.io/name: cityhall
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* Name of the Secret holding config env: existing one, or the chart-created one. */}}
{{- define "cityhall.secretName" -}}
{{- if .Values.existingSecret -}}
{{- .Values.existingSecret -}}
{{- else -}}
{{- printf "%s-config" .Release.Name -}}
{{- end -}}
{{- end -}}

{{/* Effective DATABASE_URL: explicit override, else the bundled postgres. */}}
{{- define "cityhall.databaseUrl" -}}
{{- if .Values.config.databaseUrl -}}
{{- .Values.config.databaseUrl -}}
{{- else -}}
{{- printf "postgres://cityhall:%s@%s-db:5432/cityhall" .Values.postgres.password .Release.Name -}}
{{- end -}}
{{- end -}}

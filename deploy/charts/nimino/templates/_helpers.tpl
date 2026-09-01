{{/* Standard naming/labels helpers. */}}

{{- define "nimino.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "nimino.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "nimino.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "nimino.labels" -}}
helm.sh/chart: {{ include "nimino.chart" . }}
{{ include "nimino.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: nimino
{{- end -}}

{{- define "nimino.selectorLabels" -}}
app.kubernetes.io/name: {{ include "nimino.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* Relay-specific selector: scopes the relay StatefulSet + Service so they do
     not also match the quickstart MinIO pods, which share the base
     selectorLabels but carry their own component label. */}}
{{- define "nimino.relaySelectorLabels" -}}
{{ include "nimino.selectorLabels" . }}
app.kubernetes.io/component: relay
{{- end -}}

{{- define "nimino.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "nimino.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "nimino.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}

{{/*
Name of the chart-managed Secret holding relay-identity material and any
chart-composed connection strings.
*/}}
{{- define "nimino.chartSecretName" -}}
{{- printf "%s-relay" (include "nimino.fullname" .) -}}
{{- end -}}

{{/*
The Secret name the relay should pull env from. If the operator supplied
secrets.existingSecret, use that. Otherwise use the chart-managed one.
*/}}
{{- define "nimino.envSecretName" -}}
{{- if .Values.secrets.existingSecret -}}
{{- .Values.secrets.existingSecret -}}
{{- else -}}
{{- include "nimino.chartSecretName" . -}}
{{- end -}}
{{- end -}}

{{/* Host derived from relayUrl, used as ingress default + media domain. */}}
{{- define "nimino.relayHost" -}}
{{- $url := required "relayUrl is required: set --set relayUrl=wss://your.domain" .Values.relayUrl -}}
{{- $stripped := $url | replace "wss://" "" | replace "ws://" "" | replace "https://" "" | replace "http://" "" -}}
{{- first (splitList "/" $stripped) -}}
{{- end -}}

{{/* Default media base URL: https://<host>/media derived from relayUrl. */}}
{{- define "nimino.mediaBaseUrl" -}}
{{- if .Values.mediaBaseUrl -}}
{{- .Values.mediaBaseUrl -}}
{{- else -}}
{{- printf "https://%s/media" (include "nimino.relayHost" .) -}}
{{- end -}}
{{- end -}}

{{/* Quickstart-only in-cluster service hostnames (eval profile). */}}
{{- define "nimino.minioFullname" -}}
{{- printf "%s-minio" (include "nimino.fullname" .) -}}
{{- end -}}

{{/* In-cluster MinIO endpoint, used when minio.enabled and s3.endpoint unset. */}}
{{- define "nimino.minioEndpoint" -}}
{{- printf "http://%s.%s.svc.cluster.local:9000" (include "nimino.minioFullname" .) .Release.Namespace -}}
{{- end -}}

{{/* Minimum number of relay replicas the release can run. */}}
{{- define "nimino.minimumReplicas" -}}
{{- if .Values.autoscaling.enabled -}}
{{- .Values.autoscaling.minReplicas -}}
{{- else -}}
{{- .Values.replicaCount -}}
{{- end -}}
{{- end -}}

{{/* Maximum ordinal that can be scheduled by the fixed voter set. */}}
{{- define "nimino.clusterNodeCount" -}}
{{- if .Values.autoscaling.enabled -}}
{{- .Values.autoscaling.maxReplicas -}}
{{- else -}}
{{- .Values.replicaCount -}}
{{- end -}}
{{- end -}}

{{/* Deterministic 16-byte ASCII identities, rendered as the runtime's hex IDs. */}}
{{- define "nimino.clusterVoters" -}}
{{- $root := . -}}
{{- $voters := list -}}
{{- range $ordinal := until (include "nimino.clusterNodeCount" $root | int) -}}
{{- $seed := printf "%s-%d" (include "nimino.fullname" $root) $ordinal -}}
{{- $identity := substr 0 16 (sha256sum $seed) -}}
{{- $voters = append $voters (printf "%x" $identity) -}}
{{- end -}}
{{- join "," $voters -}}
{{- end -}}

{{/* Effective huddle-audio availability. Nil means safe chart default: on for
     one replica, off for multi-pod until an SFU/shared-room story exists. */}}
{{- define "nimino.huddleAudioAvailable" -}}
{{- if kindIs "invalid" .Values.relay.huddleAudioAvailable -}}
{{- if gt (include "nimino.minimumReplicas" . | int) 1 -}}false{{- else -}}true{{- end -}}
{{- else -}}
{{- .Values.relay.huddleAudioAvailable -}}
{{- end -}}
{{- end -}}

{{/* Effective S3 endpoint: explicit s3.endpoint wins, else bundled MinIO. */}}
{{- define "nimino.s3Endpoint" -}}
{{- if .Values.s3.endpoint -}}
{{- .Values.s3.endpoint -}}
{{- else if .Values.minio.enabled -}}
{{- include "nimino.minioEndpoint" . -}}
{{- end -}}
{{- end -}}

{{- define "nimino.pairingRelaySelectorLabels" -}}
{{ include "nimino.selectorLabels" . }}
app.kubernetes.io/component: pairing-relay
{{- end -}}

{{/*
Expand the name of the chart.
*/}}
{{- define "nxcc.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "nxcc.fullname" -}}
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
{{- define "nxcc.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "nxcc.labels" -}}
helm.sh/chart: {{ include "nxcc.chart" . }}
{{ include "nxcc.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "nxcc.selectorLabels" -}}
app.kubernetes.io/name: {{ include "nxcc.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
GKE Confidential Computing annotations
*/}}
{{- define "nxcc.confidentialAnnotations" -}}
{{- $confidentialConfig := include "nxcc.effectiveConfidential" . | fromYaml }}
{{- if $confidentialConfig.enabled }}
# This annotation tells GKE Autopilot to use a Confidential Computing VM.
# It ensures the pod runs on a machine with TDX.
cloud.google.com/compute-class: "Confidential"
# For GKE Standard, you might need this instead on a specific node pool:
# cloud.google.com/gke-confidential-nodes: "true"
{{- end }}
{{- end }}


{{/*
Operator key volume mounts
*/}}
{{- define "nxcc.operatorKeyVolumeMounts" -}}
{{- $operatorConfig := include "nxcc.effectiveOperatorKey" . | fromYaml }}
{{- if $operatorConfig.enabled }}
- name: operator-key
  mountPath: /etc/nxcc/operator-keys
  readOnly: true
{{- end }}
{{- end }}

{{/*
Operator key volumes
*/}}
{{- define "nxcc.operatorKeyVolumes" -}}
{{- $operatorConfig := include "nxcc.effectiveOperatorKey" . | fromYaml }}
{{- if $operatorConfig.enabled }}
- name: operator-key
  secret:
    secretName: {{ $operatorConfig.secretName }}
    defaultMode: 0400
{{- end }}
{{- end }}

{{/*
Operator key environment variables
*/}}
{{- define "nxcc.operatorKeyEnvVars" -}}
{{- $operatorConfig := include "nxcc.effectiveOperatorKey" . | fromYaml }}
{{- if $operatorConfig.enabled }}
- name: NXCC_OPERATOR_PRIVATE_KEY_PATH
  value: "/etc/nxcc/operator-keys/private-key"
{{- end }}
{{- end }}

{{/*
Get effective operator key configuration for a component
Usage: {{ include "nxcc.effectiveOperatorKey" . }}
*/}}
{{- define "nxcc.effectiveOperatorKey" -}}
{{- $variant := .Values.nodeVariant | default "" }}
{{- $baseConfig := .Values.operatorKey }}
{{- $override := dict }}
{{- if and $variant (hasKey .Values.nodeVariations.operatorKeys $variant) }}
{{- $override = index .Values.nodeVariations.operatorKeys $variant }}
{{- end }}
{{- $result := mergeOverwrite $baseConfig $override }}
enabled: {{ $result.enabled }}
secretName: {{ $result.secretName | quote }}
createSecret: {{ $result.createSecret }}
privateKeyData: {{ $result.privateKeyData | quote }}
{{- end }}

{{/*
Get effective confidential configuration for a component
Usage: {{ include "nxcc.effectiveConfidential" . }}
*/}}
{{- define "nxcc.effectiveConfidential" -}}
{{- $variant := .Values.nodeVariant | default "" }}
{{- $baseConfig := .Values.confidential }}
{{- $override := dict }}
{{- if and $variant (hasKey .Values.nodeVariations.confidentialOverrides $variant) }}
{{- $override = index .Values.nodeVariations.confidentialOverrides $variant }}
{{- end }}
{{- $result := mergeOverwrite $baseConfig $override }}
enabled: {{ $result.enabled }}
{{- end }}

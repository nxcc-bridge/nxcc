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
{{- if .Values.confidential.enabled }}
# This annotation tells GKE Autopilot to use a Confidential Computing VM.
# It ensures the pod runs on a machine with TDX.
cloud.google.com/compute-class: "Confidential"
# For GKE Standard, you might need this instead on a specific node pool:
# cloud.google.com/gke-confidential-nodes: "true"
{{- end }}
{{- end }}

import { Address, Hex } from "viem";

export interface DsseSignatureEntry {
  keyid?: string;
  sig: string; // base64 encoded
}

export interface DsseEnvelope {
  payload: string; // base64 encoded
  payloadType: string;
  signatures: DsseSignatureEntry[];
}

export interface WorkerBundlePointer {
  source: string;
  hash?: number[];
  vm?: string;
}

export type ChainIdentifier = number | string | string[];

export interface SecretId {
  chain: ChainIdentifier;
  identity_address: Address;
  identity_id: string; // U256 as string
}

export interface WorkerManifest {
  vm?: string;
  bundle: WorkerBundlePointer;
  identities: [SecretId, string][];
  userdata: Record<string, any>;
}

export interface WorkerBundlePayload {
  vm: string;
  executable: string; // base64 encoded string
  metadata: Record<string, string>;
}

interface BaseWorkerEvent {
  handler: string;
}

export interface LaunchWorkerEvent extends BaseWorkerEvent {
  kind: "launch";
}

export interface HttpRequestWorkerEvent extends BaseWorkerEvent {
  kind: "http_request";
}

export interface Web3WorkerEvent extends BaseWorkerEvent {
  kind: "web3_event";
  chain: ChainIdentifier;
  address?: Address[];
  topics?: Hex[][];
  gateways?: string[];
}

export type ScheduleMode = "rate";
export type ScheduleCatchUp = "skip" | "coalesce" | "queue";

export interface ScheduledEventPolicy {
  catch_up?: ScheduleCatchUp;
  max_lateness_ms?: number;
  jitter_budget_ms?: number;
}

export interface ScheduledWorkerEvent extends BaseWorkerEvent {
  kind: "scheduled";
  period_ms: number;
  mode?: ScheduleMode;
  phase_ms?: number;
  start_at?: string;
  end_at?: string;
  max_occurrences?: number;
  policy?: ScheduledEventPolicy;
}

export type WorkerEvent =
  | LaunchWorkerEvent
  | HttpRequestWorkerEvent
  | Web3WorkerEvent
  | ScheduledWorkerEvent;

export interface WorkOrderPayload {
  id: string;
  worker: WorkerManifest;
  events: WorkerEvent[];
}

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
}

export type ChainIdentifier = number | string | string[];

export interface SecretId {
  chain: ChainIdentifier;
  identity_address: Address;
  identity_id: string; // U256 as string
}

export interface WorkerManifest {
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
  address: Address[];
  topics: Hex[][];
}

export type WorkerEvent = LaunchWorkerEvent | HttpRequestWorkerEvent | Web3WorkerEvent;

export interface WorkOrderPayload {
  id: string;
  worker: WorkerManifest;
  events: WorkerEvent[];
}

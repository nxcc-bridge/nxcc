import { privateKeyToAccount, signMessage } from "viem/accounts";
import { Hex, toBytes } from "viem";
import { DsseEnvelope, DsseSignatureEntry } from "./types";

function pae(type: string, payload: string): Uint8Array {
  const typeBytes = toBytes(type);
  const payloadBytes = toBytes(payload);

  const paeParts = [
    toBytes("DSSEv1"),
    toBytes(" "),
    toBytes(String(typeBytes.length)),
    toBytes(" "),
    typeBytes,
    toBytes(" "),
    toBytes(String(payloadBytes.length)),
    toBytes(" "),
    payloadBytes,
  ];

  const totalLength = paeParts.reduce((sum, p) => sum + p.length, 0);
  const result = new Uint8Array(totalLength);

  let offset = 0;
  for (const part of paeParts) {
    result.set(part, offset);
    offset += part.length;
  }

  return result;
}

export async function signDsse(
  payload: string,
  payloadType: string,
  signerKey: Hex,
): Promise<DsseEnvelope> {
  const account = privateKeyToAccount(signerKey);
  const dataToSign = pae(payloadType, payload);

  const signature = await signMessage({
    privateKey: signerKey,
    message: { raw: dataToSign },
  });

  const sigEntry: DsseSignatureEntry = {
    keyid: account.address,
    sig: Buffer.from(signature.substring(2), "hex").toString("base64"),
  };

  const envelope: DsseEnvelope = {
    payload: Buffer.from(payload).toString("base64"),
    payloadType,
    signatures: [sigEntry],
  };

  return envelope;
}

export function createUnsignedDsse(payload: string, payloadType: string): DsseEnvelope {
  const dummySig: DsseSignatureEntry = {
    keyid: "bench-key",
    sig: Buffer.from("benches").toString("base64"),
  };
  return {
    payload: Buffer.from(payload).toString("base64"),
    payloadType,
    signatures: [dummySig],
  };
}

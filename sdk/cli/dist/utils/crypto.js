"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.signDsse = signDsse;
exports.createUnsignedDsse = createUnsignedDsse;
const accounts_1 = require("viem/accounts");
const viem_1 = require("viem");
function pae(type, payload) {
    const typeBytes = (0, viem_1.toBytes)(type);
    const payloadBytes = (0, viem_1.toBytes)(payload);
    const paeParts = [
        (0, viem_1.toBytes)("DSSEv1"),
        (0, viem_1.toBytes)(" "),
        (0, viem_1.toBytes)(String(typeBytes.length)),
        (0, viem_1.toBytes)(" "),
        typeBytes,
        (0, viem_1.toBytes)(" "),
        (0, viem_1.toBytes)(String(payloadBytes.length)),
        (0, viem_1.toBytes)(" "),
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
async function signDsse(payload, payloadType, signerKey) {
    const account = (0, accounts_1.privateKeyToAccount)(signerKey);
    const dataToSign = pae(payloadType, payload);
    const signature = await (0, accounts_1.signMessage)({
        privateKey: signerKey,
        message: { raw: dataToSign },
    });
    const sigEntry = {
        keyid: account.address,
        sig: Buffer.from(signature.substring(2), "hex").toString("base64"),
    };
    const envelope = {
        payload: Buffer.from(payload).toString("base64"),
        payloadType,
        signatures: [sigEntry],
    };
    return envelope;
}
function createUnsignedDsse(payload, payloadType) {
    return {
        payload: Buffer.from(payload).toString("base64"),
        payloadType,
        signatures: [],
    };
}

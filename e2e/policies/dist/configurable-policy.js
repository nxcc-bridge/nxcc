var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __commonJS = (cb, mod) => function __require() {
  return mod || (0, cb[__getOwnPropNames(cb)[0]])((mod = { exports: {} }).exports, mod), mod.exports;
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
  // If the importer is in node compatibility mode or this is not an ESM
  // file that has been converted to a CommonJS file using a Babel-
  // compatible transform (i.e. "__esModule" has not been set), then set
  // "default" to the CommonJS "module.exports" for node compatibility.
  isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
  mod
));

// node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/crypto/deriveKey.js
var require_deriveKey = __commonJS({
  "node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/crypto/deriveKey.js"(exports) {
    "use strict";
    Object.defineProperty(exports, "__esModule", { value: true });
    exports.deriveKey = deriveKey;
    async function deriveKey(base, purpose, path, options) {
      const hashAlgorithm = options?.hash ?? "SHA-256";
      const hashOutputSizes = {
        "SHA-256": 32,
        "SHA-384": 48,
        "SHA-512": 64
      };
      const hashOutputSize = hashOutputSizes[hashAlgorithm];
      if (!hashOutputSize) {
        throw new Error(`Unsupported hash algorithm: ${hashAlgorithm}. Supported: SHA-256, SHA-384, SHA-512`);
      }
      const hkdfMaxOutput = 255 * hashOutputSize;
      const length = options?.length ?? hashOutputSize;
      if (!base) {
        throw new Error("Base key is required");
      }
      if (!(base instanceof CryptoKey)) {
        throw new Error("Base must be a CryptoKey instance");
      }
      if (base.type !== "secret") {
        console.warn('Base key type is not "secret", may not be suitable for HKDF operations');
      }
      if (typeof purpose !== "string" || !purpose.trim()) {
        throw new Error("Purpose must be a non-empty string");
      }
      if (!Number.isInteger(length) || length <= 0 || length > hkdfMaxOutput) {
        throw new Error(`Invalid length: ${length}. Must be an integer between 1-${hkdfMaxOutput} bytes (HKDF-${hashAlgorithm} limit)`);
      }
      if (!Array.isArray(path)) {
        throw new Error("Path must be an array");
      }
      if (options?.salt !== void 0 && !(options.salt instanceof Uint8Array)) {
        throw new Error("Salt must be a Uint8Array");
      }
      const salt = options?.salt ?? new Uint8Array();
      const textEncoder = new TextEncoder();
      const MAX_INFO_SIZE = 1024 * 1024;
      function u32be(n) {
        if (n < 0 || n > 4294967295 || !Number.isInteger(n)) {
          throw new Error(`Invalid length: ${n}. Must be a non-negative integer \u2264 2^32-1`);
        }
        const buffer = new ArrayBuffer(4);
        const view = new DataView(buffer);
        view.setUint32(0, n, false);
        return new Uint8Array(buffer);
      }
      const infoParts = [];
      const encodedPurpose = textEncoder.encode(purpose);
      infoParts.push(u32be(encodedPurpose.byteLength));
      infoParts.push(encodedPurpose);
      for (let i = 0; i < path.length; i++) {
        const item = path[i];
        if (item === null || item === void 0) {
          throw new Error(`Path item at index ${i} cannot be null or undefined`);
        }
        if (typeof item !== "string" && !(item instanceof Uint8Array)) {
          throw new Error(`Path item at index ${i} must be a string or Uint8Array`);
        }
        const data = typeof item === "string" ? textEncoder.encode(item) : item;
        if (data.byteLength === 0) {
          throw new Error(`Path item at index ${i} cannot be empty`);
        }
        if (data.byteLength > 4294967295) {
          throw new Error(`Path item at index ${i} is too large (${data.byteLength} bytes)`);
        }
        infoParts.push(u32be(data.byteLength));
        infoParts.push(data);
      }
      let totalInfoLength = 0;
      for (const part of infoParts) {
        totalInfoLength += part.byteLength;
        if (totalInfoLength > MAX_INFO_SIZE) {
          throw new Error(`Total info size exceeds maximum ${MAX_INFO_SIZE} bytes`);
        }
      }
      const concatenatedInfo = new Uint8Array(totalInfoLength);
      let offset = 0;
      for (const part of infoParts) {
        concatenatedInfo.set(part, offset);
        offset += part.byteLength;
      }
      try {
        const derivedBits = await crypto.subtle.deriveBits({
          name: "HKDF",
          hash: hashAlgorithm,
          salt,
          info: concatenatedInfo
        }, base, length * 8);
        return new Uint8Array(derivedBits);
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : String(error);
        throw new Error(`Key derivation failed: ${errorMessage}`);
      }
    }
  }
});

// node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/crypto/index.js
var require_crypto = __commonJS({
  "node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/crypto/index.js"(exports) {
    "use strict";
    Object.defineProperty(exports, "__esModule", { value: true });
    exports.deriveKey = void 0;
    var deriveKey_1 = require_deriveKey();
    Object.defineProperty(exports, "deriveKey", { enumerable: true, get: function() {
      return deriveKey_1.deriveKey;
    } });
  }
});

// node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/policy/index.js
var require_policy = __commonJS({
  "node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/policy/index.js"(exports) {
    "use strict";
    Object.defineProperty(exports, "__esModule", { value: true });
    exports.policy = policy2;
    function policy2(handler) {
      return {
        async fetch(request) {
          const url = new URL(request.url);
          const handlerName = url.pathname.startsWith("/") ? url.pathname.substring(1) : url.pathname;
          if (handlerName !== "_policy") {
            console.error(`Policy worker received unexpected handler: ${handlerName}`);
            return new Response(`Policy worker: unexpected handler ${handlerName}`, { status: 400 });
          }
          try {
            const contextsArray = await request.json();
            console.log(`Processing ${contextsArray.length} policy execution requests`);
            const decisions = await handler(contextsArray);
            if (!Array.isArray(decisions) || decisions.length !== contextsArray.length) {
              throw new Error(`Policy handler must return an array of ${contextsArray.length} boolean decisions`);
            }
            return new Response(JSON.stringify(decisions), {
              status: 200,
              headers: { "content-type": "application/json; charset=utf-8" }
            });
          } catch (err) {
            console.error("Policy worker error:", err);
            return new Response(JSON.stringify({
              error: "Policy worker execution failed",
              message: err instanceof Error ? err.message : String(err)
            }), {
              status: 500,
              headers: { "content-type": "application/json; charset=utf-8" }
            });
          }
        }
      };
    }
  }
});

// node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/worker/index.js
var require_worker = __commonJS({
  "node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/worker/index.js"(exports) {
    "use strict";
    Object.defineProperty(exports, "__esModule", { value: true });
    exports.worker = worker;
    function convertToResponse(result) {
      if (result instanceof Response) {
        return result;
      }
      if (result === void 0 || result === null) {
        return new Response(null, { status: 204 });
      }
      if (result instanceof Error) {
        return new Response(result.message, { status: 500 });
      }
      try {
        return new Response(JSON.stringify(result), {
          status: 200,
          headers: { "Content-Type": "application/json" }
        });
      } catch (error) {
        return new Response(String(result), { status: 200 });
      }
    }
    function worker(config) {
      const handlers = {};
      if (config.launch) {
        handlers.launch = config.launch;
      }
      for (const [handlerName, handler] of Object.entries(config)) {
        if (handlerName !== "fetch" && handlerName !== "launch" && typeof handler === "function") {
          handlers[handlerName] = handler;
        }
      }
      return {
        async fetch(request, env, ctx) {
          const context = {
            userdata: env.USER_CONFIG || {},
            env
          };
          if (request.method === "POST") {
            let vmInvocationPayload;
            try {
              vmInvocationPayload = await request.clone().json();
            } catch (error) {
            }
            if (vmInvocationPayload && typeof vmInvocationPayload === "object" && typeof vmInvocationPayload.handler === "string") {
              const handler = handlers[vmInvocationPayload.handler];
              if (handler) {
                try {
                  const result = await handler(vmInvocationPayload.event_payload, context);
                  return convertToResponse(result);
                } catch (error) {
                  return convertToResponse(error);
                }
              }
              return new Response(`No handler for ${vmInvocationPayload.handler}`, {
                status: 404
              });
            }
          }
          if (config.fetch) {
            try {
              const result = await config.fetch(request, context);
              return convertToResponse(result);
            } catch (error) {
              return convertToResponse(error);
            }
          }
          return new Response("HTTP handler not implemented", { status: 501 });
        }
      };
    }
  }
});

// node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/index.js
var require_dist = __commonJS({
  "node_modules/.pnpm/@nxcc+sdk@file+..+..+sdk+lib/node_modules/@nxcc/sdk/dist/index.js"(exports) {
    "use strict";
    var __createBinding = exports && exports.__createBinding || (Object.create ? (function(o, m, k, k2) {
      if (k2 === void 0) k2 = k;
      var desc = Object.getOwnPropertyDescriptor(m, k);
      if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
        desc = { enumerable: true, get: function() {
          return m[k];
        } };
      }
      Object.defineProperty(o, k2, desc);
    }) : (function(o, m, k, k2) {
      if (k2 === void 0) k2 = k;
      o[k2] = m[k];
    }));
    var __setModuleDefault = exports && exports.__setModuleDefault || (Object.create ? (function(o, v) {
      Object.defineProperty(o, "default", { enumerable: true, value: v });
    }) : function(o, v) {
      o["default"] = v;
    });
    var __importStar = exports && exports.__importStar || /* @__PURE__ */ (function() {
      var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function(o2) {
          var ar = [];
          for (var k in o2) if (Object.prototype.hasOwnProperty.call(o2, k)) ar[ar.length] = k;
          return ar;
        };
        return ownKeys(o);
      };
      return function(mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) {
          for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        }
        __setModuleDefault(result, mod);
        return result;
      };
    })();
    Object.defineProperty(exports, "__esModule", { value: true });
    exports.worker = exports.policy = exports.crypto = void 0;
    exports.crypto = __importStar(require_crypto());
    var index_1 = require_policy();
    Object.defineProperty(exports, "policy", { enumerable: true, get: function() {
      return index_1.policy;
    } });
    var index_2 = require_worker();
    Object.defineProperty(exports, "worker", { enumerable: true, get: function() {
      return index_2.worker;
    } });
  }
});

// configurable-policy.ts
var import_sdk = __toESM(require_dist());
var configurable_policy_default = (0, import_sdk.policy)((requests) => {
  const enforceValidation = globalThis.USER_CONFIG?.enforce_validation !== false;
  const policyMode = enforceValidation ? "TDX Validation" : "Permissive";
  console.log(`\u{1F510} ${policyMode} Policy executing with ${requests.length} requests`);
  console.log(`   enforce_validation: ${enforceValidation}`);
  console.log(`   globalThis.USER_CONFIG:`, JSON.stringify(globalThis.USER_CONFIG, null, 2));
  return requests.map((request, i) => {
    console.log(`
\u{1F50D} DEBUG: Full request ${i}:`, JSON.stringify(request, null, 2));
    console.log(`\u{1F50D} DEBUG: env_report:`, JSON.stringify(request.env_report, null, 2));
    console.log(`\u{1F50D} DEBUG: env_report keys:`, Object.keys(request.env_report || {}));
    console.log(`\u{1F50D} DEBUG: attestation_claims:`, JSON.stringify(request.attestation_claims, null, 2));
    const bundleHash = request.consumer?.bundle_hash ? Array.from(request.consumer.bundle_hash).map((b) => b.toString(16).padStart(2, "0")).join("").substring(0, 16) : "unknown";
    console.log(`
\u{1F50D} Evaluating request ${i} for consumer: ${bundleHash}`);
    if (!enforceValidation) {
      console.log(`   \u2705 APPROVED: ${bundleHash} - Permissive mode accepts all`);
      return true;
    }
    const claims = request.attestation_claims;
    if (!claims) {
      console.log(`   \u274C DENIED: No attestation claims for ${bundleHash}`);
      return false;
    }
    if (!claims.measurements || claims.measurements.length === 0) {
      console.log(`   \u274C DENIED: No measurements for ${bundleHash}`);
      return false;
    }
    for (const measurement of claims.measurements) {
      if (measurement.val && measurement.val.length > 0) {
        const isAllZeros = measurement.val.every((byte) => byte === 0);
        if (!isAllZeros) {
          console.log(`   \u2705 APPROVED: ${bundleHash} - Valid TDX measurements`);
          return true;
        }
      }
    }
    console.log(`   \u274C DENIED: ${bundleHash} - All measurements are zero (simulation)`);
    return false;
  });
});
export {
  configurable_policy_default as default
};

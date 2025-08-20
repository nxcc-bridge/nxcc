/**
 * Standardized attestation claims following IETF EAT (RFC 9711)
 * These are the verified claims extracted from the attestation
 */
export interface AttestationClaims {
  /** Issued-at time of the evidence production or verification moment */
  iat: number;
  /** Verifier challenge to prevent replay (if used) */
  eat_nonce?: Uint8Array;
  /** Stable device/realm identity */
  ueid?: Uint8Array;
  /** Manufacturer identifier */
  oemid?: string;
  /** Hardware model descriptor */
  hwmodel?: string;
  /** Hardware/firmware version string */
  hwversion?: string;
  /** Debug/production mode: 0=debug disabled (production), 4=debug enabled */
  dbgstat: number;
  /** OEM-authorized secure boot active */
  oemboot?: boolean;
  /** Product or component name of the attested software root */
  swname?: string;
  /** Version string of the attested software root */
  swversion?: string;
  /** Cryptographic measurements relevant to trust decisions */
  measurements: Array<{
    /** Hash value */
    val: Uint8Array;
    /** Hash algorithm: "sha-256", "sha-384", or "sha-512" */
    alg: string;
    /** Category: "boot", "firmware", "kernel", "initrd", "vmm", "application", "policy", etc. */
    measurement_type?: string;
    /** Vendor information */
    vendor?: string;
    /** Version information */
    version?: string;
  }>;
  /** Proof-of-possession key bound to this attested state */
  cnf?: {
    jwk?: {
      /** Key type: "EC", "RSA", "OKP" */
      kty: string;
      /** Curve for EC/OKP keys: "P-256", "P-384", "P-521", "X25519", "Ed25519" */
      crv?: string;
      /** X coordinate (for EC keys) or raw key (for OKP) */
      x?: string;
      /** Y coordinate (for EC keys) */
      y?: string;
    };
    cose_key?: Uint8Array;
  };
  /** Intended use for the token/key (typically 5 for proof-of-possession) */
  intuse?: number;
  /** Seconds since last boot according to the attested environment */
  uptime?: number;
  /** Number of boots observed */
  bootcount?: number;
  /** Per-boot unique random seed to distinguish boot instances */
  bootseed?: Uint8Array;
  /** URI-like identifier of the interpretation profile for platform specifics */
  eat_profile: string;
}

/**
 * Policy execution request from the NXCC platform.
 * Contains attestation data and other context for authorization decisions.
 */
export interface PolicyExecutionRequest {
  env_report: {
    node_id: string;
    attestation: any; // Raw attestation report for backward compatibility
  };
  secret_ids: string[];
  consumer: any;
  /** Standardized attestation claims extracted from the verified attestation.
   * Available when the attestation system successfully verifies the report.
   * Policies should check for the presence of this field to ensure attestation was verified.
   */
  attestation_claims?: AttestationClaims;
}

/**
 * Policy handler function type.
 * Receives an array of policy execution requests and returns an array of boolean decisions.
 */
export type PolicyHandler = (requests: PolicyExecutionRequest[]) => boolean[] | Promise<boolean[]>;

/**
 * Creates a policy worker that handles the NXCC policy execution protocol.
 *
 * @param handler - Function that receives policy execution requests and returns authorization decisions
 * @returns A worker object compatible with the Cloudflare Workers runtime
 *
 * @example
 * ```typescript
 * import { policy } from '@nxcc/sdk';
 *
 * // Allow-all policy
 * export default policy((requests) => {
 *   return requests.map(() => true);
 * });
 *
 * // Custom authorization logic
 * export default policy((requests) => {
 *   return requests.map(request => {
 *     // Your authorization logic here
 *     return request.env_report.node_id.startsWith('trusted-');
 *   });
 * });
 * ```
 */
export function policy(handler: PolicyHandler) {
  return {
    async fetch(request: Request): Promise<Response> {
      const url = new URL(request.url);
      const handlerName = url.pathname.startsWith("/") ? url.pathname.substring(1) : url.pathname;

      // Policy workers must handle the "_policy" endpoint
      if (handlerName !== "_policy") {
        console.error(`Policy worker received unexpected handler: ${handlerName}`);
        return new Response(`Policy worker: unexpected handler ${handlerName}`, { status: 400 });
      }

      try {
        // Parse the policy execution requests
        const contextsArray = await request.json();
        console.log(`Processing ${contextsArray.length} policy execution requests`);

        // Call the user's policy handler
        const decisions = await handler(contextsArray);

        // Validate that the handler returned the correct number of decisions
        if (!Array.isArray(decisions) || decisions.length !== contextsArray.length) {
          throw new Error(
            `Policy handler must return an array of ${contextsArray.length} boolean decisions`,
          );
        }

        return new Response(JSON.stringify(decisions), {
          status: 200,
          headers: { "content-type": "application/json; charset=utf-8" },
        });
      } catch (err) {
        console.error("Policy worker error:", err);
        return new Response(
          JSON.stringify({
            error: "Policy worker execution failed",
            message: err instanceof Error ? err.message : String(err),
          }),
          {
            status: 500,
            headers: { "content-type": "application/json; charset=utf-8" },
          },
        );
      }
    },
  };
}

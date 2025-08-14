/**
 * Policy execution request from the NXCC platform.
 * Contains attestation data and other context for authorization decisions.
 */
export interface PolicyExecutionRequest {
  env_report: {
    node_id: string;
    attestation: any; // RATS/IEATS attestation token claims
  };
  secret_ids: string[];
  consumer: any;
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
          throw new Error(`Policy handler must return an array of ${contextsArray.length} boolean decisions`);
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
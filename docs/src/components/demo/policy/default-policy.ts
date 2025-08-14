/**
 * Default NXCC Policy Worker for Demo
 * 
 * This is a basic policy that accepts all authorization requests.
 * It demonstrates the policy worker interface:
 * - Receives Vec<PolicyExecutionRequest> as JSON
 * - Returns Vec<bool> as JSON (true = authorized, false = denied)
 */

export default {
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
      console.log(`Demo policy processing ${contextsArray.length} authorization requests`);

      // Default policy: approve all requests
      // In production, you would examine each context's attestation data
      const resultsArray = contextsArray.map(() => true);

      return new Response(JSON.stringify(resultsArray), {
        status: 200,
        headers: { "content-type": "application/json; charset=utf-8" },
      });
    } catch (err) {
      console.error("Demo policy worker error:", err);
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
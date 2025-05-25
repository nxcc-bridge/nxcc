export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const handlerName = url.pathname.startsWith("/") ? url.pathname.substring(1) : url.pathname;

    if (handlerName !== "_policy") {
      console.error(`Policy worker received unexpected handler: ${handlerName}`);
      return new Response(`Policy worker: unexpected handler ${handlerName}`, { status: 400 });
    }

    try {
      // The body is Vec<PolicyExecutionRequest> directly, not wrapped in VmEventInvocation
      const contextsArray = await request.json(); 
      // console.log("Parsed JSON object (contextsArray):", contextsArray);

      // For each context in the input array, decide if it's approved.
      // This mock policy approves all contexts.
      const resultsArray = contextsArray.map(context => true); 

      return new Response(JSON.stringify(resultsArray), {
        status: 200,
        headers: { "content-type": "application/json; charset=utf-8" },
      });
    } catch (err) {
      console.error("Policy worker error:", err); // Log error with context
      return new Response(
        JSON.stringify({
          error: "Policy worker execution failed",
          message: err.message,
          stack: err.stack, // Include stack for better debugging
        }),
        {
          status: 500, // Internal server error from policy worker
          headers: { "content-type": "application/json; charset=utf-8" },
        },
      );
    }
  },
};

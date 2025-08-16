let scheduledEventCount = 0;
const scheduledEventResults = [];

async function handleLaunch(eventPayload, env) {
  console.log("Scheduled event worker received Launch event. Initializing...");
  scheduledEventCount = 0;
  return new Response(JSON.stringify({ 
    message: "Scheduled event worker initialized",
    count: scheduledEventCount 
  }), {
    headers: { "Content-Type": "application/json" },
  });
}

async function handleScheduledTick(eventPayload, env) {
  scheduledEventCount++;
  const timestamp = Date.now();
  const result = {
    count: scheduledEventCount,
    timestamp: timestamp,
    message: `Scheduled event ${scheduledEventCount} processed`
  };
  
  scheduledEventResults.push(result);
  console.log(`Scheduled event ${scheduledEventCount} processed at ${timestamp}`);
  
  return new Response(JSON.stringify(result), {
    headers: { "Content-Type": "application/json" },
  });
}

async function handleGetCount(eventPayload, env) {
  console.log(`Returning scheduled event count: ${scheduledEventCount}`);
  return new Response(JSON.stringify({ 
    count: scheduledEventCount,
    results: scheduledEventResults
  }), {
    headers: { "Content-Type": "application/json" },
  });
}

const handlers = {
  launch: handleLaunch,
  scheduledTick: handleScheduledTick,
  getCount: handleGetCount,
};

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    let handlerName = url.pathname.startsWith("/")
      ? url.pathname.substring(1)
      : url.pathname;

    // Handle HTTP GET requests to /count
    if (request.method === "GET" && handlerName === "count") {
      return new Response(JSON.stringify({ 
        count: scheduledEventCount,
        results: scheduledEventResults
      }), {
        headers: { "Content-Type": "application/json" },
      });
    }

    // Handle VM event invocations
    const vmInvocationPayload = await request.json();
    console.log(
      `Scheduled worker received VmInvocationPayload for path ${url.pathname}: ${JSON.stringify(vmInvocationPayload)}`,
    );

    const actualHandler =
      handlers[handlerName] || handlers[vmInvocationPayload.handler];
    if (actualHandler) {
      return actualHandler(vmInvocationPayload.event_payload, env);
    } else {
      console.error(
        `No handler found for '${handlerName}' or '${vmInvocationPayload.handler}'`,
      );
      return new Response(
        `No handler for ${handlerName || vmInvocationPayload.handler}`,
        { status: 404 },
      );
    }
  },
};
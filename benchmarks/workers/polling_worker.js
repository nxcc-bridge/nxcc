async function backgroundTask(env) {
  const interval_ms = env.USER_CONFIG?.interval_ms || 1000;
  const url = env.USER_CONFIG?.url || "http://anvil:8545";

  async function doPoll() {
    while (true) {
      const start = Date.now();
      await fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          method: "eth_blockNumber",
          params: [],
          id: 1,
        }),
      });
      const elapsed = Date.now() - start;
      if (elapsed > 3 * interval_ms)
        throw new Error("interval greatly exceeded");
      const delay = Math.max(0, interval_ms - elapsed);
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
  }

  await doPoll();
}

async function handleLaunch(eventPayload, env) {
  await backgroundTask(env);
}

const handlers = {
  launch: handleLaunch,
};

export default {
  async fetch(request, env, ctx) {
    const vmInvocationPayload = await request.json();
    const handler = handlers[vmInvocationPayload.handler];

    if (handler) {
      return handler(vmInvocationPayload.event_payload, env);
    } else {
      return new Response(`No handler for ${vmInvocationPayload.handler}`, {
        status: 404,
      });
    }
  },
};

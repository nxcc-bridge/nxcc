async function backgroundTask(env) {
  const concurrency = env.USER_CONFIG?.concurrency || 5;
  const delay = env.USER_CONFIG?.delay_ms || 100;

  async function doFetch() {
    while (true) {
      try {
        await fetch(`https://httpbin.org/delay/${delay / 1000}`);
      } catch (e) {}
    }
  }

  const promises = [];
  for (let i = 0; i < concurrency; i++) {
    promises.push(doFetch());
  }

  await Promise.all(promises);
}

async function handleLaunch(eventPayload, env) {
  backgroundTask(env);
  return new Response("IO-bound worker launched and running in background.", {
    status: 200,
  });
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

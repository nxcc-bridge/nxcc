async function handleLaunch(eventPayload, env) {
  const target = env.USER_CONFIG?.iterations || 1000000;

  function isPrime(n) {
    if (n < 2) return false;
    for (let i = 2; i * i <= n; i++) {
      if (n % i === 0) return false;
    }
    return true;
  }

  const start = Date.now();
  let primeCount = 0;

  // This blocks the main thread continuously
  for (let i = 2; i < target; i++) {
    if (isPrime(i)) primeCount++;
  }

  const duration = Date.now() - start;

  return new Response(
    `Found ${primeCount} primes up to ${target} in ${duration}ms`,
    { status: 200 },
  );
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

export default {
  async handleLaunch(event_payload, env) {
    // This handler is called for the "launch" event.
    // The main purpose of this worker is to test secret derivation.
    console.log("Policy/Secret test worker: handleLaunch called.");

    if (!env.THE_SECRET) {
      console.error(
        "env.THE_SECRET not found in worker environment for handleLaunch",
      );
      return new Response("env.THE_SECRET not found (launch)", { status: 500 });
    }

    try {
      // env.THE_SECRET is a CryptoKey with deriveBits capability
      // We’ll do a minimal HKDF deriveBits with empty salt/info to get 128 bits.
      const derivedBuffer = await crypto.subtle.deriveBits(
        {
          name: "HKDF",
          hash: "SHA-256",
          salt: new Uint8Array(), // empty
          info: new Uint8Array(), // empty
        },
        env.THE_SECRET,
        128, // number of bits to derive
      );
      // Turn ArrayBuffer → Uint8Array → binary string → base64
      const bytes = new Uint8Array(derivedBuffer);
      let binary = "";
      for (let b of bytes) {
        binary += String.fromCharCode(b);
      }
      const base64Derived = btoa(binary);
      // Log for test harness
      console.log(`DERIVED_BASE64: ${base64Derived}`);
      return new Response(base64Derived, {
        status: 200,
        headers: { "Content-Type": "text/plain" },
      });
    } catch (e) {
      console.error("Error deriving bits:", e);
      return new Response(`Worker error (launch): ${e.message}`, {
        status: 500,
      });
    }
  },

  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const handlerName = url.pathname.startsWith("/")
      ? url.pathname.substring(1)
      : url.pathname;
    const vmInvocationPayload = await request.json();

    if (handlerName === "launch" && this.handleLaunch) {
      // The payload for launch is VmEventInvocation { handler: "launch", event_payload: {} }
      // So we pass vmInvocationPayload.event_payload to the specific handler.
      return this.handleLaunch(vmInvocationPayload.event_payload, env);
    } else {
      console.error(`Unknown handler or path: ${handlerName}`);
      return new Response(`Unknown handler: ${handlerName}`, { status: 404 });
    }
  },
};

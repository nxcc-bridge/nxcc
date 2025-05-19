export default {
  async fetch(_req, env) {
    if (!env.THE_SECRET) {
      console.error("env.THE_SECRET not found in worker environment");
      return new Response("env.THE_SECRET not found", { status: 500 });
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
      return new Response(`Worker error: ${e.message}`, { status: 500 });
    }
  },
};

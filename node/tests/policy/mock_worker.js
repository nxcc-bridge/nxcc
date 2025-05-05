// This mock worker always approves all contexts passed to it.
// It expects the input payload to be CBOR-encoded Vec<PolicyExecutionRequest>
// and returns CBOR-encoded Vec<bool> where each element is true.

// NOTE: In a real workerd environment, you'd use native APIs.
// This is a simplified representation for the test setup.
// We assume the VMM handles the CBOR encoding/decoding based on the
// RunnerService implementation.

export default {
  async fetch(request, env, ctx) {
    try {
      // In a real scenario, we'd need a CBOR library or WASM module.
      // For this test, the VMM mock will handle the logic.
      // This worker code itself isn't actually executed by workerd in the test,
      // but the VMM mock simulates its behavior.
      // The important part is that the runner expects a CBOR Vec<bool> back.
      console.log("Mock policy worker invoked (this log won't appear in test)");
      // Simulate returning 'true' for all contexts. The VMM mock will construct the actual CBOR.
      return new Response("Policy Approved (Mock)", { status: 200 });
    } catch (e) {
      return new Response(`Error in mock worker: ${e}`, { status: 500 });
    }
  }
}

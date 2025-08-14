/**
 * Default NXCC Policy Worker
 *
 * This is a basic policy that accepts all authorization requests.
 * It demonstrates the policy worker interface using the NXCC SDK.
 *
 * Customize this policy to implement your authorization logic.
 */

import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  // Default policy: approve all requests
  // In a real policy, you would examine each request's attestation data
  // and make authorization decisions based on your security requirements
  return requests.map(() => true);
});

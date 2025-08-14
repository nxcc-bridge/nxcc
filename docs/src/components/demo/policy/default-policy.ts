/**
 * Default NXCC Policy Worker for Demo
 *
 * This is a basic policy that accepts all authorization requests.
 * It demonstrates the policy worker interface using the NXCC SDK.
 */

import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  console.log(
    `Demo policy processing ${requests.length} authorization requests`,
  );

  // Default policy: approve all requests
  // In production, you would examine each request's attestation data
  return requests.map(() => true);
});

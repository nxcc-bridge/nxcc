// Configurable Policy
// Policy that can either validate TDX measurements or be permissive based on userdata

import { policy } from '@nxcc/sdk';

export default policy((requests) => {
  // Policy configuration from userdata
  const enforceValidation = globalThis.USER_CONFIG?.enforce_validation !== false;
  const policyMode = enforceValidation ? "TDX Validation" : "Permissive";
  
  console.log(`🔐 ${policyMode} Policy executing with ${requests.length} requests`);
  console.log(`   enforce_validation: ${enforceValidation}`);
  console.log(`   globalThis.USER_CONFIG:`, JSON.stringify(globalThis.USER_CONFIG, null, 2));
  
  return requests.map((request, i) => {
    console.log(`\n🔍 DEBUG: Full request ${i}:`, JSON.stringify(request, null, 2));
    console.log(`🔍 DEBUG: env_report:`, JSON.stringify(request.env_report, null, 2));
    console.log(`🔍 DEBUG: env_report keys:`, Object.keys(request.env_report || {}));
    console.log(`🔍 DEBUG: attestation_claims:`, JSON.stringify(request.attestation_claims, null, 2));
    
    // There's no node_id field in env_report - use consumer bundle hash as identifier
    const bundleHash = request.consumer?.bundle_hash ? 
      Array.from(request.consumer.bundle_hash).map(b => b.toString(16).padStart(2, '0')).join('').substring(0, 16) : 
      'unknown';
    console.log(`\n🔍 Evaluating request ${i} for consumer: ${bundleHash}`);
    
    // If permissive mode, always approve
    if (!enforceValidation) {
      console.log(`   ✅ APPROVED: ${bundleHash} - Permissive mode accepts all`);
      return true;
    }
    
    // TDX validation mode - check measurements
    const claims = request.attestation_claims;
    if (!claims) {
      console.log(`   ❌ DENIED: No attestation claims for ${bundleHash}`);
      return false;
    }
    
    if (!claims.measurements || claims.measurements.length === 0) {
      console.log(`   ❌ DENIED: No measurements for ${bundleHash}`);
      return false;
    }
    
    // Check for non-zero measurements (indicates real TDX hardware)
    for (const measurement of claims.measurements) {
      if (measurement.val && measurement.val.length > 0) {
        const isAllZeros = measurement.val.every(byte => byte === 0);
        if (!isAllZeros) {
          console.log(`   ✅ APPROVED: ${bundleHash} - Valid TDX measurements`);
          return true;
        }
      }
    }
    
    console.log(`   ❌ DENIED: ${bundleHash} - All measurements are zero (simulation)`);
    return false;
  });
});
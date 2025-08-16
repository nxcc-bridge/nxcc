// Attestation Policy Worker
// Tests policy execution with standardized attestation claims

// Handler function for policy evaluation
function _policy(contexts) {
    console.log("Attestation policy executing with", contexts.length, "contexts");
    
    const results = [];
    
    for (let i = 0; i < contexts.length; i++) {
        const context = contexts[i];
        console.log(`Evaluating context ${i} for node: ${context.env_report.node_id}`);
        
        // Check if attestation claims are available
        if (context.attestation_claims) {
            const claims = context.attestation_claims;
            console.log(`Claims available - Platform: ${claims.platform_id}`);
            console.log(`Debug disabled: ${claims.debug_disabled}`);
            console.log(`Security version: ${claims.security_version_number}`);
            
            // Policy decision based on attestation claims
            const isValid = claims.debug_disabled && claims.security_version_number > 0;
            
            if (isValid) {
                console.log(`✓ Context ${i} APPROVED - Valid attestation claims`);
                results.push(true);
            } else {
                console.log(`✗ Context ${i} DENIED - Invalid attestation claims`);
                results.push(false);
            }
        } else {
            console.log(`✗ Context ${i} DENIED - No attestation claims available`);
            results.push(false);
        }
    }
    
    console.log("Policy results:", results);
    return results;
}

// Export for the VM
globalThis._policy = _policy;

console.log("Attestation policy worker loaded");
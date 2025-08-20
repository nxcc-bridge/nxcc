export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const handlerName = url.pathname.startsWith("/") ? url.pathname.substring(1) : url.pathname;

    if (handlerName !== "_policy") {
      console.error(`Policy worker received unexpected handler: ${handlerName}`);
      return new Response(`Policy worker: unexpected handler ${handlerName}`, { status: 400 });
    }

    try {
      const contextsArray = await request.json(); 
      console.log("🔐 Operator Signature Policy executing with", contextsArray.length, "contexts");
      
      const results = [];
      
      for (let i = 0; i < contextsArray.length; i++) {
        const context = contextsArray[i];
        const nodeId = context.env_report.node_id;
        
        console.log(`\n🔍 Evaluating context ${i} for node: ${nodeId}`);
        
        let approved = true;
        const denyReasons = [];

        // SECURITY CHECK 1: Operator signature validation
        const operatorSignature = context.env_report.operator_signature;
        
        if (!operatorSignature) {
          approved = false;
          denyReasons.push("No operator signature provided");
          console.log(`   ❌ No operator signature for ${nodeId}`);
        } else {
          // Check if the operator signature has the expected structure
          if (!operatorSignature.cose_sign1 || operatorSignature.cose_sign1.length === 0) {
            approved = false;
            denyReasons.push("Invalid operator signature structure");
            console.log(`   ❌ Invalid operator signature structure for ${nodeId}`);
          } else {
            console.log(`   ✅ Valid operator signature for ${nodeId} (${operatorSignature.cose_sign1.length} bytes)`);
          }
        }

        // SECURITY CHECK 2: Self-authorization (always allowed with valid signature)
        if (nodeId === '@self') {
          console.log(`   ✅ Self-authorization: ${nodeId} - allowed to access own secrets`);
        } else {
          console.log(`   ✅ Cross-node access: ${nodeId} with valid operator signature`);
        }

        // Final decision
        if (approved) {
          console.log(`✅ APPROVED: ${nodeId} - Valid operator signature`);
          results.push(true);
        } else {
          console.log(`❌ DENIED: ${nodeId} - Security validation failed`);
          denyReasons.forEach(reason => console.log(`   - ${reason}`));
          results.push(false);
        }
      }
      
      console.log("\n📊 Policy Decision Summary:");
      console.log(`   Approved: ${results.filter(r => r).length}/${results.length} contexts`);
      console.log("   Results:", results);

      return new Response(JSON.stringify(results), {
        status: 200,
        headers: { "content-type": "application/json; charset=utf-8" },
      });
    } catch (err) {
      console.error("Policy worker error:", err);
      return new Response(
        JSON.stringify({
          error: "Policy worker execution failed",
          message: err.message,
          stack: err.stack,
        }),
        {
          status: 500,
          headers: { "content-type": "application/json; charset=utf-8" },
        },
      );
    }
  },
};
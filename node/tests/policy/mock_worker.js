export default {
  async fetch(request, env, ctx) {
    try {
      const rawText = await request.text();
      console.log("Raw request body:", rawText);
      const data = JSON.parse(rawText);
      console.log("Parsed JSON object:", data);

      return new Response(JSON.stringify([true], null, 2), { // TODO: return the number of trues as in the input
        status: 200,
        headers: { "content-type": "application/json; charset=utf-8" },
      });
    } catch (err) {
      console.error("JSON parse error:", err);
      return new Response(
        JSON.stringify({
          error: "Invalid JSON in request body",
          message: err.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json; charset=utf-8" },
        },
      );
    }
  },
};

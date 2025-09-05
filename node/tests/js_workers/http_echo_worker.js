export default {
  async fetch(request, env, ctx) {
    console.log(`HTTP Echo Worker: Received ${request.method} request to ${request.url}`);
    
    const url = new URL(request.url);
    let bodyContent = "";
    if (request.body) {
      bodyContent = await request.text();
      console.log(`HTTP Echo Worker: Request body: ${bodyContent}`);
    }
    
    const responseBody = {
      message: "HTTP Echo Worker Response",
      method: request.method,
      pathname: url.pathname,
      searchParams: Object.fromEntries(url.searchParams),
      headers: Object.fromEntries(request.headers),
      body: bodyContent,
    };
    
    console.log(`HTTP Echo Worker: Sending response for ${url.pathname}`);
    
    return new Response(JSON.stringify(responseBody), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  },
};

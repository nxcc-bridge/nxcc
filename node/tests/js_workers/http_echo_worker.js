export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    let bodyContent = "";
    if (request.body) {
      bodyContent = await request.text();
    }
    const responseBody = {
      message: "HTTP Echo Worker Response",
      method: request.method,
      pathname: url.pathname,
      searchParams: Object.fromEntries(url.searchParams),
      headers: Object.fromEntries(request.headers),
      body: bodyContent,
    };
    return new Response(JSON.stringify(responseBody), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  },
};

let count = 0;

function handleRequest(request) {
  if (request.method === "POST") {
    count++;
    return new Response(null, { status: 204 });
  } else if (request.method === "GET") {
    return new Response(count.toString(), { status: 200 });
  }
  return new Response("Not Found", { status: 404 });
}

export default {
  fetch: handleRequest,
};

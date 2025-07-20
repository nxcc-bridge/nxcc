export default {
  async fetch(request: Request, env: any, ctx: any): Promise<Response> {
    console.log('Hello from the worker!');
    return new Response('Hello, nXCC!');
  },

  async launch(event_payload: any, env: any): Promise<Response> {
    console.log('Worker launched!');
    return new Response('Launch event handled.');
  },
};

import { worker } from '@nxcc/sdk';

export default worker({
  async launch(eventPayload, { userdata }) {
    console.log('Worker launched!', eventPayload);
    return new Response('Launch event handled.');
  },

  async fetch(request, { userdata }) {
    console.log('HTTP request received:', request.method, new URL(request.url).pathname);
    return new Response('Hello from nXCC worker!');
  }
});

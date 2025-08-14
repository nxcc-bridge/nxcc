/**
 * Worker event context passed to all handlers.
 * Contains userdata from the worker configuration and other environment data.
 */
export interface WorkerContext {
  userdata: any;
  env: any;
}

/**
 * Handler function type for worker events.
 * Receives event payload and context, returns a Response.
 */
export type WorkerHandler = (
  eventPayload: any,
  context: WorkerContext,
) => Promise<Response> | Response;

/**
 * HTTP handler function type for worker HTTP requests.
 * Receives Request and context, returns a Response.
 */
export type WorkerHttpHandler = (
  request: Request,
  context: WorkerContext,
) => Promise<Response> | Response;

/**
 * Launch handler function type for worker launch events.
 * Receives event payload and context, returns a Response.
 */
export type WorkerLaunchHandler = (
  eventPayload: any,
  context: WorkerContext,
) => Promise<Response> | Response;

/**
 * Worker configuration object.
 */
export interface WorkerConfig {
  /** Handler for launch events */
  launch?: WorkerLaunchHandler;
  /** Handler for HTTP requests (via invoke_http) */
  fetch?: WorkerHttpHandler;
  /** Custom event handlers */
  [handlerName: string]: WorkerHandler | WorkerHttpHandler | WorkerLaunchHandler | undefined;
}

/**
 * Creates a worker that handles the NXCC worker execution protocol.
 *
 * @param config - Configuration object with event handlers
 * @returns A worker object compatible with the Cloudflare Workers runtime
 *
 * @example
 * ```typescript
 * import { worker } from '@nxcc/sdk';
 *
 * export default worker({
 *   async launch({ userdata, env }) {
 *     console.log('Worker launched!', userdata);
 *     return new Response('Launch event handled.');
 *   },
 *
 *   async fetch(request, { userdata, env }) {
 *     return new Response('Hello from nXCC worker!');
 *   },
 *
 *   async myCustomHandler(eventPayload, { userdata, env }) {
 *     console.log('Custom event:', eventPayload);
 *     return new Response('Custom event handled.');
 *   }
 * });
 * ```
 */
export function worker(config: WorkerConfig) {
  // Build handlers object from config
  const handlers: Record<string, WorkerHandler> = {};

  // Add launch handler if provided
  if (config.launch) {
    handlers.launch = config.launch;
  }

  // Add custom handlers (exclude 'fetch' as it's handled separately)
  for (const [handlerName, handler] of Object.entries(config)) {
    if (handlerName !== "fetch" && handlerName !== "launch" && typeof handler === "function") {
      handlers[handlerName] = handler as WorkerHandler;
    }
  }

  return {
    async fetch(request: Request, env: any, ctx: any): Promise<Response> {
      const context: WorkerContext = {
        userdata: env.USER_CONFIG || {},
        env,
      };

      // Handle direct HTTP requests (via invoke_http)
      const url = new URL(request.url);
      if (request.method !== "POST" || url.pathname === "/") {
        if (config.fetch) {
          return config.fetch(request, context);
        } else {
          return new Response("HTTP handler not implemented", { status: 501 });
        }
      }

      // Handle VM invocations (via invoke_worker)
      try {
        const vmInvocationPayload = await request.json();
        const handler = handlers[vmInvocationPayload.handler];

        if (handler) {
          return handler(vmInvocationPayload.event_payload, context);
        } else {
          return new Response(`No handler for ${vmInvocationPayload.handler}`, {
            status: 404,
          });
        }
      } catch (error) {
        // If JSON parsing fails, treat as regular HTTP request
        if (config.fetch) {
          return config.fetch(request, context);
        } else {
          return new Response("HTTP handler not implemented", { status: 501 });
        }
      }
    },
  };
}

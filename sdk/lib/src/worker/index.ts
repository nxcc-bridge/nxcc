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
 * Receives event payload and context, can return various types that will be automatically converted.
 */
export type WorkerHandler<T = any> = (eventPayload: any, context: WorkerContext) => Promise<T> | T;

/**
 * HTTP handler function type for worker HTTP requests.
 * Receives Request and context, can return various types that will be automatically converted.
 */
export type WorkerHttpHandler<T = any> = (
  request: Request,
  context: WorkerContext,
) => Promise<T> | T;

/**
 * Launch handler function type for worker launch events.
 * Receives event payload and context, can return void or other types.
 */
export type WorkerLaunchHandler = (
  eventPayload: any,
  context: WorkerContext,
) => Promise<void | any> | void | any;

/**
 * Custom event handler - typically doesn't return anything meaningful (void/undefined becomes 204).
 */
export type CustomEventHandler = (
  eventPayload: any,
  context: WorkerContext,
) => Promise<void | any> | void | any;

/**
 * Worker configuration object with specialized handlers for launch/fetch and custom event handlers.
 */
export interface WorkerConfig {
  /** Handler for launch events - can return void or any value */
  launch?: WorkerLaunchHandler;
  /** Handler for HTTP requests (via invoke_http) - can return any value */
  fetch?: WorkerHttpHandler;
  /** Custom event handlers - can return any value that will be converted to Response */
  [handlerName: string]: CustomEventHandler | WorkerLaunchHandler | WorkerHttpHandler | undefined;
}

/**
 * Converts various return types to a Response object.
 * - Response: returned as-is
 * - object/array: JSON stringified with 200 status
 * - undefined/null/void: 204 No Content
 * - Error: 500 with error message
 */
function convertToResponse(result: any): Response {
  // If it's already a Response, return as-is
  if (result instanceof Response) {
    return result;
  }

  // If undefined, null, or explicitly void, return 204
  if (result === undefined || result === null) {
    return new Response(null, { status: 204 });
  }

  // If it's an Error, return 500
  if (result instanceof Error) {
    return new Response(result.message, { status: 500 });
  }

  // For objects/arrays/primitives, JSON stringify with 200
  try {
    return new Response(JSON.stringify(result), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  } catch (error) {
    // If JSON.stringify fails, convert to string
    return new Response(String(result), { status: 200 });
  }
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
 *   // Launch handler - can return void or any value
 *   async launch(eventPayload, { userdata, env }) {
 *     console.log('Worker launched!', eventPayload, userdata);
 *     // No return needed - automatically returns 204
 *   },
 *
 *   // Fetch handler - return objects are JSON stringified with 200
 *   async fetch(request, { userdata, env }) {
 *     return { message: 'Hello from nXCC worker!', method: request.method };
 *   },
 *
 *   // Custom handlers - return values are automatically converted
 *   async myCustomHandler(eventPayload, { userdata, env }) {
 *     console.log('Custom event:', eventPayload);
 *     // Return object automatically becomes JSON response with 200
 *     return { status: 'handled', timestamp: Date.now() };
 *   },
 *
 *   async anotherHandler(eventPayload, context) {
 *     // Can still return Response objects directly
 *     return new Response('Custom response', { status: 201 });
 *   }
 * });
 * ```
 */
export function worker(config: WorkerConfig) {
  // Build handlers object from config
  const handlers: Record<string, CustomEventHandler> = {};

  // Add launch handler if provided
  if (config.launch) {
    handlers.launch = config.launch;
  }

  // Add custom handlers (exclude 'fetch' as it's handled separately)
  for (const [handlerName, handler] of Object.entries(config)) {
    if (handlerName !== "fetch" && handlerName !== "launch" && typeof handler === "function") {
      handlers[handlerName] = handler as CustomEventHandler;
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
      if (request.method !== "POST") {
        if (config.fetch) {
          try {
            const result = await config.fetch(request, context);
            return convertToResponse(result);
          } catch (error) {
            return convertToResponse(error);
          }
        } else {
          return new Response("HTTP handler not implemented", { status: 501 });
        }
      }

      // Handle VM invocations (via invoke_worker)
      try {
        const vmInvocationPayload = await request.json();
        const handler = handlers[vmInvocationPayload.handler];

        if (handler) {
          try {
            const result = await handler(vmInvocationPayload.event_payload, context);
            return convertToResponse(result);
          } catch (error) {
            return convertToResponse(error);
          }
        } else {
          return new Response(`No handler for ${vmInvocationPayload.handler}`, {
            status: 404,
          });
        }
      } catch (error) {
        // If JSON parsing fails, treat as regular HTTP request
        if (config.fetch) {
          try {
            const result = await config.fetch(request, context);
            return convertToResponse(result);
          } catch (fetchError) {
            return convertToResponse(fetchError);
          }
        } else {
          return new Response("HTTP handler not implemented", { status: 501 });
        }
      }
    },
  };
}

import { worker } from "./index";

describe("worker function", () => {
  const mockEnv = {
    USER_CONFIG: { test: "config" },
  };
  const mockCtx = {};

  describe("automatic response conversion", () => {
    it("should convert objects to JSON responses with 200 status", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          return { message: "hello", timestamp: 123 };
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      expect(response.headers.get("content-type")).toBe("application/json");

      const data = await response.json();
      expect(data).toEqual({ message: "hello", timestamp: 123 });
    });

    it("should return 204 for undefined/null/void returns", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          // Return undefined (void function)
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(204);
      expect(await response.text()).toBe("");
    });

    it("should return Response objects as-is", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          return new Response("custom response", {
            status: 201,
            headers: { "X-Custom": "value" },
          });
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(201);
      expect(response.headers.get("X-Custom")).toBe("value");
      expect(await response.text()).toBe("custom response");
    });

    it("should convert errors to 500 responses", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          throw new Error("Something went wrong");
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(500);
      expect(await response.text()).toBe("Something went wrong");
    });

    it("should handle primitive values", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          return "plain string";
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      expect(await response.json()).toBe("plain string");
    });

    it("should handle arrays", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          return [1, 2, 3];
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      expect(await response.json()).toEqual([1, 2, 3]);
    });

    it("should route POST requests without handler metadata to the fetch handler", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          const body = await request.json();
          return {
            method: request.method,
            pathname: new URL(request.url).pathname,
            body,
            context,
          };
        },
      });

      const request = new Request("http://example.com/transfer", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ amount: "1" }),
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      const data = await response.json();
      expect(data).toEqual({
        method: "POST",
        pathname: "/transfer",
        body: { amount: "1" },
        context: { userdata: { test: "config" }, env: mockEnv },
      });
    });
  });

  describe("custom event handlers", () => {
    it("should handle custom event returning void with 204 status", async () => {
      const testWorker = worker({
        async customEvent(eventPayload, context) {
          console.log("Processing custom event:", eventPayload);
          // No return - custom events typically don't return meaningful data
        },
      });

      const requestBody = {
        handler: "customEvent",
        event_payload: { test: "data" },
      };
      const request = new Request("http://example.com/", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(requestBody),
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(204);
    });

    it("should handle custom event that does return something", async () => {
      const testWorker = worker({
        async customEvent(eventPayload, context) {
          // Edge case where custom event does return something
          return { handled: true, payload: eventPayload };
        },
      });

      const requestBody = {
        handler: "customEvent",
        event_payload: { test: "data" },
      };
      const request = new Request("http://example.com/", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(requestBody),
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      const data = await response.json();
      expect(data).toEqual({ handled: true, payload: { test: "data" } });
    });

    it("should handle custom event errors", async () => {
      const testWorker = worker({
        async errorEvent(eventPayload, context) {
          throw new Error("Custom error");
        },
      });

      const requestBody = {
        handler: "errorEvent",
        event_payload: { test: "data" },
      };
      const request = new Request("http://example.com/", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(requestBody),
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(500);
      expect(await response.text()).toBe("Custom error");
    });

    it("should return 404 for unknown handlers", async () => {
      const testWorker = worker({
        async knownHandler(eventPayload, context) {
          return { handled: true };
        },
      });

      const requestBody = {
        handler: "unknownHandler",
        event_payload: { test: "data" },
      };
      const request = new Request("http://example.com/", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(requestBody),
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(404);
      expect(await response.text()).toBe("No handler for unknownHandler");
    });
  });

  describe("launch handler", () => {
    it("should handle launch events with automatic response conversion", async () => {
      const testWorker = worker({
        async launch(eventPayload, context) {
          return { launched: true, payload: eventPayload };
        },
      });

      const requestBody = {
        handler: "launch",
        event_payload: { test: "launch data" },
      };
      const request = new Request("http://example.com/", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(requestBody),
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      const data = await response.json();
      expect(data).toEqual({ launched: true, payload: { test: "launch data" } });
    });

    it("should handle launch handler returning void", async () => {
      const testWorker = worker({
        async launch(eventPayload, context) {
          // No return (void)
        },
      });

      const requestBody = {
        handler: "launch",
        event_payload: { test: "launch data" },
      };
      const request = new Request("http://example.com/", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(requestBody),
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(204);
    });
  });

  describe("worker context", () => {
    it("should pass correct context to handlers", async () => {
      let capturedContext: any;

      const testWorker = worker({
        async fetch(request, context) {
          capturedContext = context;
          return { received: "ok" };
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      await testWorker.fetch(request, mockEnv, mockCtx);

      expect(capturedContext).toEqual({
        userdata: { test: "config" },
        env: mockEnv,
      });
    });
  });

  describe("fallback behavior", () => {
    it("should handle malformed JSON in POST requests", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          return { fallback: true };
        },
      });

      const request = new Request("http://example.com/", {
        method: "POST",
        body: "invalid json",
      });

      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      const data = await response.json();
      expect(data).toEqual({ fallback: true });
    });

    it("should return 501 when no fetch handler is provided for HTTP requests", async () => {
      const testWorker = worker({
        async customEvent(eventPayload, context) {
          return { handled: true };
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(501);
      expect(await response.text()).toBe("HTTP handler not implemented");
    });
  });

  describe("JSON serialization edge cases", () => {
    it("should handle objects that fail JSON.stringify by converting to string", async () => {
      const testWorker = worker({
        async fetch(request, context) {
          const obj = {};
          // Create a circular reference that will cause JSON.stringify to throw
          (obj as any).circular = obj;
          return obj;
        },
      });

      const request = new Request("http://example.com/", { method: "GET" });
      const response = await testWorker.fetch(request, mockEnv, mockCtx);

      expect(response.status).toBe(200);
      expect(await response.text()).toBe("[object Object]");
    });
  });
});

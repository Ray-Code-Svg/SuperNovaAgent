import { describe, expect, it } from "vitest";

import { LocalRuntimeProtocolClient } from "../../protocol/generated/client";

describe("Local runtime protocol client auth", () => {
  it("sends the runtime token header on JSON and stream requests", async () => {
    const seenHeaders: Array<Record<string, string>> = [];
    const fetchImpl: typeof fetch = async (_input, init) => {
      seenHeaders.push(headersToRecord(init?.headers));
      return new Response(
        JSON.stringify({
          protocol_version: "supernova.local_runtime.v1",
          schema_version: "supernova.protocol.response.v1",
          request_id: "req_test",
          workspace_id: "ws_test",
          resource: "test",
          data: {
            status: "ready",
            runtime_layer: "rust_product_runtime",
            workspace_id: "ws_test",
            uptime_ms: 1
          }
        }),
        { status: 200 }
      );
    };
    const client = new LocalRuntimeProtocolClient({
      baseUrl: "http://runtime.test",
      runtimeToken: "token_123",
      fetchImpl
    });

    await client.runtimeHealth();
    await client.chatTurnStream("chat_1", {
      message: "hello",
      context_pack: null,
      context_pack_id: null,
      model_config: null,
      session_id: null,
      source_guidance: null
    });

    expect(seenHeaders).toHaveLength(2);
    expect(seenHeaders[0]["x-supernova-runtime-token"]).toBe("token_123");
    expect(seenHeaders[1]["x-supernova-runtime-token"]).toBe("token_123");
    expect(seenHeaders[1]["content-type"]).toBe("application/json");
  });
});

function headersToRecord(headers: HeadersInit | undefined): Record<string, string> {
  if (!headers) return {};
  const entries =
    headers instanceof Headers
      ? Array.from(headers.entries())
      : Array.isArray(headers)
        ? headers
        : Object.entries(headers);
  return Object.fromEntries(
    entries.map(([key, value]) => [key.toLowerCase(), String(value)])
  );
}

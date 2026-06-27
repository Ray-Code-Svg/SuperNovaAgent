import { describe, expect, it } from "vitest";

import { isNearScrollBottom, scrollTopForBottom } from "./useStickyScroll";

describe("useStickyScroll helpers", () => {
  it("treats a stream inside the bottom threshold as pinned", () => {
    expect(
      isNearScrollBottom({
        scrollHeight: 1200,
        scrollTop: 645,
        clientHeight: 500
      } as HTMLDivElement)
    ).toBe(true);
  });

  it("treats a stream above the bottom threshold as detached", () => {
    expect(
      isNearScrollBottom({
        scrollHeight: 1200,
        scrollTop: 600,
        clientHeight: 500
      } as HTMLDivElement)
    ).toBe(false);
  });

  it("calculates the exact bottom scrollTop without relying on browser clamping", () => {
    expect(scrollTopForBottom({ scrollHeight: 1200, clientHeight: 500 } as HTMLDivElement)).toBe(700);
    expect(scrollTopForBottom({ scrollHeight: 300, clientHeight: 500 } as HTMLDivElement)).toBe(0);
  });
});

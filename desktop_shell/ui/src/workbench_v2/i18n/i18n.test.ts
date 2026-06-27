import { describe, expect, it } from "vitest";

import { messages } from "./i18n";

describe("i18n message catalog", () => {
  it("keeps zh-CN and en-US keys in sync", () => {
    expect(Object.keys(messages["zh-CN"]).sort()).toEqual(Object.keys(messages["en-US"]).sort());
  });
});

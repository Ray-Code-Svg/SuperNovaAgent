import { describe, expect, it } from "vitest";

import { isWorkbenchExternalUrl, normalizeLatexMathDelimiters } from "./MessageMarkdown";

describe("normalizeLatexMathDelimiters", () => {
  it("normalizes common LaTeX inline and display delimiters for remark-math", () => {
    expect(normalizeLatexMathDelimiters("令 \\(f(x)=x^2\\)。\n\\[\\int_0^1 x dx\\]")).toBe(
      "令 $f(x)=x^2$。\n\n$$\n\\int_0^1 x dx\n$$\n"
    );
  });

  it("does not rewrite delimiters inside fenced code blocks", () => {
    const source = "```tex\n\\(x\\)\n```\n\n正文 \\(y\\)";

    expect(normalizeLatexMathDelimiters(source)).toBe("```tex\n\\(x\\)\n```\n\n正文 $y$");
  });
});

describe("isWorkbenchExternalUrl", () => {
  it("allows default browser URL schemes used in Workbench messages", () => {
    expect(isWorkbenchExternalUrl("https://127.0.0.1:8501/")).toBe(true);
    expect(isWorkbenchExternalUrl("http://127.0.0.1:8000/")).toBe(true);
    expect(isWorkbenchExternalUrl("mailto:team@example.com")).toBe(true);
    expect(isWorkbenchExternalUrl("tel:+123456789")).toBe(true);
  });

  it("rejects relative links and executable script schemes", () => {
    expect(isWorkbenchExternalUrl("/relative/path")).toBe(false);
    expect(isWorkbenchExternalUrl("#section")).toBe(false);
    expect(isWorkbenchExternalUrl("javascript:alert(1)")).toBe(false);
    expect(isWorkbenchExternalUrl("file:///C:/Users/test.txt")).toBe(false);
  });
});

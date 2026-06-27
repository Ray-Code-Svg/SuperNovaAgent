import ReactMarkdown from "react-markdown";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { openExternalUrl } from "../protocol/runtimeClient";

import "katex/dist/katex.min.css";

interface MessageMarkdownProps {
  text: string;
  mathStable?: boolean;
  normalizeMathDelimiters?: boolean;
}

export function MessageMarkdown({ text, mathStable = true, normalizeMathDelimiters = false }: MessageMarkdownProps) {
  const markdownText = mathStable && normalizeMathDelimiters ? normalizeLatexMathDelimiters(text) : text;
  return (
    <div className="sn-markdown">
      <ReactMarkdown
        remarkPlugins={mathStable ? [remarkGfm, remarkMath] : [remarkGfm]}
        rehypePlugins={mathStable ? [rehypeKatex] : []}
        components={{
          a: ({ node: _node, href, children, ...props }) => (
            <a
              {...props}
              href={href}
              rel="noreferrer"
              target={isWorkbenchExternalUrl(href) ? "_blank" : undefined}
              onClick={(event) => {
                if (!href || href.startsWith("#")) return;
                event.preventDefault();
                if (!isWorkbenchExternalUrl(href)) return;
                void openExternalUrl(href).catch((error) => {
                  console.warn("Failed to open Workbench external URL", error);
                });
              }}
            >
              {children}
            </a>
          )
        }}
      >
        {markdownText}
      </ReactMarkdown>
    </div>
  );
}

export function isWorkbenchExternalUrl(href: string | null | undefined) {
  if (!href) return false;
  const trimmed = href.trim();
  const schemeMatch = /^([a-z][a-z0-9+.-]*):/i.exec(trimmed);
  if (!schemeMatch) return false;
  return ["http", "https", "mailto", "tel"].includes(schemeMatch[1].toLowerCase());
}

export function normalizeLatexMathDelimiters(text: string) {
  const segments = text.split(/(```[\s\S]*?```|~~~[\s\S]*?~~~)/g);
  return segments
    .map((segment) => {
      if (segment.startsWith("```") || segment.startsWith("~~~")) return segment;
      return segment
        .replace(/\\\[([\s\S]*?)\\\]/g, (_, expression: string) => `\n$$\n${expression.trim()}\n$$\n`)
        .replace(/\\\(([\s\S]*?)\\\)/g, (_, expression: string) => `$${expression.trim()}$`);
    })
    .join("");
}

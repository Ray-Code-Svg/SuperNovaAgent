import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";

import type { AppSettings, ProviderApiRecord } from "../../protocol/generated/types";
import { useWorkbenchUiStore } from "../state/uiStore";
import { OnboardingGuide } from "./OnboardingGuide";

describe("OnboardingGuide", () => {
  beforeEach(() => {
    useWorkbenchUiStore.setState({ language: "en-US" });
  });

  it("renders the missing API key setup path and core Workbench guide sections", () => {
    const html = renderToStaticMarkup(
      <OnboardingGuide
        settings={appSettings([{ provider: "deepseek", api_key_configured: false }])}
        onOpenProviderSettings={() => {}}
        onOpenAppearanceSettings={() => {}}
      />
    );

    expect(html).toContain("API key missing");
    expect(html).toContain("https://platform.deepseek.com/api_keys");
    expect(html).toContain("href=\"https://platform.deepseek.com/api_keys\"");
    expect(html).toContain("What SuperNova Agent does");
    expect(html).toContain("Start using SuperNova RC0");
    expect(html).toContain("AI Agent workbench for a local workspace");
    expect(html).toContain("Use TASK for longer work");
    expect(html.indexOf("What SuperNova Agent does")).toBeLessThan(html.indexOf("Provider API setup"));
    expect(html).toContain("Provider API");
    expect(html).toContain("Workspace and Containers");
    expect(html).toContain("Chat and TASK modes");
    expect(html).toContain("/chat");
    expect(html).toContain("/task");
    expect(html).toContain("Agent capabilities and limits");
    expect(html).toContain("collect many files into a file set");
    expect(html).toContain("traceable records");
    expect(html).toContain("DOCX, Workbook, PDF, and CSV");
    expect(html).toContain("time-limited commands");
    expect(html).toContain("blocking preview step");
    expect(html).toContain("Model and Context configuration");
    expect(html).toContain("recent Chat turns");
    expect(html).toContain("recent TASK runs");
    expect(html).toContain("Summary/Ref only/Full");
    expect(html).toContain("context-window token estimates");
    expect(html).not.toContain("Approvals and recovery");
    expect(html).toContain("Settings, diagnostics, archive");
  });

  it("renders configured provider readiness without the missing-key warning note", () => {
    const html = renderToStaticMarkup(
      <OnboardingGuide
        settings={appSettings([{ provider: "deepseek", api_key_configured: true }])}
        onOpenProviderSettings={() => {}}
        onOpenAppearanceSettings={() => {}}
      />
    );

    expect(html).toContain("API key configured");
    expect(html).not.toContain("The guide does not store or test keys.");
  });

  it("renders zh-CN guide copy from the active UI language", () => {
    useWorkbenchUiStore.setState({ language: "zh-CN" });

    const html = renderToStaticMarkup(
      <OnboardingGuide
        settings={appSettings([{ provider: "deepseek", api_key_configured: false }])}
        onOpenProviderSettings={() => {}}
        onOpenAppearanceSettings={() => {}}
      />
    );

    expect(html).toContain("开始使用 SuperNova（RC0 版本）");
    expect(html).toContain("SuperNova Agent 能做什么");
    expect(html).toContain("面向本地工作区的 AI Agent 工作台");
    expect(html).toContain("用 TASK 处理较长工作");
    expect(html.indexOf("SuperNova Agent 能做什么")).toBeLessThan(html.indexOf("Provider API 配置"));
    expect(html).toContain("Provider API 配置");
    expect(html).toContain("Chat 与 TASK 模式");
    expect(html).toContain("/chat");
    expect(html).toContain("/task");
    expect(html).toContain("Agent 能力边界说明");
    expect(html).toContain("把一批文件收成文件集");
    expect(html).toContain("可追踪记录");
    expect(html).toContain("DOCX、Workbook、PDF、CSV");
    expect(html).toContain("有时限的命令");
    expect(html).toContain("preview 阻塞确认");
    expect(html).not.toContain("Kernel 注册能力");
    expect(html).not.toContain("receipt");
    expect(html).not.toContain("rollback evidence");
    expect(html).not.toContain("bounded terminal");
    expect(html).toContain("模型配置与上下文配置");
    expect(html).toContain("最近 Chat 轮数");
    expect(html).toContain("最近 TASK 运行");
    expect(html).toContain("摘要/仅引用/完整");
  });
});

function appSettings(providers: ProviderApiRecord[]): AppSettings {
  return {
    provider_api: { providers },
    data_paths: {
      app_config_root: "C:/SuperNova/config",
      app_state_root: "C:/SuperNova/state",
      workspace_registry_path: "C:/SuperNova/state/workspaces.json"
    },
    appearance: {
      language: "en-US",
      theme: "dark"
    }
  };
}

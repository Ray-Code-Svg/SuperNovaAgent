import { useEffect, type ReactNode } from "react";
import { FluentProvider, webDarkTheme, webLightTheme } from "@fluentui/react-components";

import { useWorkbenchUiStore } from "../state/uiStore";

export function FluentAppShell({ children }: { children: ReactNode }) {
  const theme = useWorkbenchUiStore((state) => state.theme);

  useEffect(() => {
    document.documentElement.dataset.snTheme = theme;
  }, [theme]);

  return <FluentProvider theme={theme === "dark" ? webDarkTheme : webLightTheme}>{children}</FluentProvider>;
}

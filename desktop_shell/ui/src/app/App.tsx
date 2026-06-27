import { QueryClientProvider } from "@tanstack/react-query";

import { WorkbenchV2 } from "../workbench_v2/main/WorkbenchV2";
import { queryClient } from "../workbench_v2/protocol/queryClient";

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <WorkbenchV2 />
    </QueryClientProvider>
  );
}

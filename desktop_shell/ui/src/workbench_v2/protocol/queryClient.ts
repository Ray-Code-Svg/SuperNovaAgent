import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchInterval: 5_000,
      retry: 1,
      staleTime: 1_000
    }
  }
});

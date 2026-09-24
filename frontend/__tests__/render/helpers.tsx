import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react-native";
import type { ReactElement, ReactNode } from "react";
import { WeatherClearThemeProvider } from "../../src/theme/WeatherClearThemeProvider";

const clients: QueryClient[] = [];

// No retries, and no gc timers left running to keep Jest alive after the run.
afterEach(() => {
  for (const client of clients.splice(0)) client.clear();
});

/** Render inside the app's query client and theme providers. */
export function renderWithProviders(ui: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { gcTime: Infinity } },
  });
  clients.push(client);
  // As a wrapper, the providers survive rerender().
  const Providers = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>
      <WeatherClearThemeProvider>{children}</WeatherClearThemeProvider>
    </QueryClientProvider>
  );
  return render(ui, { wrapper: Providers });
}

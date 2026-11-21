/// <reference types="vite/client" />
interface ImportMetaEnv {
  readonly VITE_POSTHOG_KEY?: string;
  readonly VITE_POSTHOG_HOST?: string;
  readonly VITE_UMAMI_HOST?: string;
  readonly VITE_UMAMI_WEBSITE_ID?: string;
  readonly VITE_ANALYTICS_RECORDER_OWNER?: "posthog" | "umami";
}

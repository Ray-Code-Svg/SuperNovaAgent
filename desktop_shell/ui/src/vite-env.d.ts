/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_SUPERNOVA_TRANSPORT?: "mock" | "live-http" | "tauri";
  readonly VITE_SUPERNOVA_API_BASE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

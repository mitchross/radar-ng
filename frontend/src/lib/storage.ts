import { createMMKV } from "react-native-mmkv";

export const storage = createMMKV({ id: "radar-ng" });

export function getString(key: string, fallback: string): string {
  try {
    return storage.getString(key) ?? fallback;
  } catch {
    return fallback;
  }
}

export function setString(key: string, value: string): void {
  try {
    storage.set(key, value);
  } catch {}
}

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

/** Synchronous Storage-shaped adapter for the react-query persister. */
export const queryCacheStorage = {
  getItem(key: string): string | null {
    try {
      return storage.getString(key) ?? null;
    } catch {
      return null;
    }
  },
  setItem(key: string, value: string): void {
    try {
      storage.set(key, value);
    } catch {}
  },
  removeItem(key: string): void {
    try {
      storage.remove(key);
    } catch {}
  },
};

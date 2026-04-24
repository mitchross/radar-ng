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

export function getBoolean(key: string, fallback: boolean): boolean {
  try {
    const val = storage.getBoolean(key);
    return val !== undefined ? val : fallback;
  } catch {
    return fallback;
  }
}

export function setBoolean(key: string, value: boolean): void {
  try {
    storage.set(key, value);
  } catch {}
}

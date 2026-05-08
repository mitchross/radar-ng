export type LocationMode = "device" | "city";

export interface SelectedPlace {
  id: number;
  name: string;
  latitude: number;
  longitude: number;
  admin1?: string;
  country?: string;
  countryCode?: string;
}

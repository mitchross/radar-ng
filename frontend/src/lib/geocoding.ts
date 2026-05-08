import type { SelectedPlace } from "../types/location";

const GEOCODING_URL = "https://geocoding-api.open-meteo.com/v1/search";

interface OpenMeteoPlace {
  id: number;
  name: string;
  latitude: number;
  longitude: number;
  admin1?: string;
  country?: string;
  country_code?: string;
}

interface OpenMeteoResponse {
  results?: OpenMeteoPlace[];
  error?: boolean;
  reason?: string;
}

export async function searchCities(query: string): Promise<SelectedPlace[]> {
  const name = query.trim();
  if (name.length < 2) return [];

  const params = new URLSearchParams({
    name,
    count: "8",
    language: "en",
    format: "json",
  });
  const response = await fetch(`${GEOCODING_URL}?${params.toString()}`);
  const body = (await response.json()) as OpenMeteoResponse;

  if (!response.ok || body.error) {
    throw new Error(body.reason || "City search failed");
  }

  return (body.results ?? []).map((place) => ({
    id: place.id,
    name: place.name,
    latitude: place.latitude,
    longitude: place.longitude,
    admin1: place.admin1,
    country: place.country,
    countryCode: place.country_code,
  }));
}

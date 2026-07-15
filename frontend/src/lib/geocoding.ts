import type { SelectedPlace } from "../types/location";

const GEOCODING_URL = "https://geocoding-api.open-meteo.com/v1/search";
const NOMINATIM_URL = "https://nominatim.openstreetmap.org";

export async function reverseGeocode(
  latitude: number,
  longitude: number,
): Promise<SelectedPlace | null> {
  const params = new URLSearchParams({
    format: "json",
    addressdetails: "1",
    limit: "1",
  });
  const response = await fetch(
    `${NOMINATIM_URL}/reverse?lat=${latitude}&lon=${longitude}&${params.toString()}`,
    {
      headers: {
        // Nominatim requires a User-Agent
        "User-Agent": "radar-ng/1.1",
      },
    },
  );
  const body = await response.json();

  if (!response.ok || !body.display_name) {
    return null;
  }

  const address = body.address || {};
  return {
    id: body.place_id ?? 0,
    name: address.city || address.town || address.village || address.municipality || body.display_name.split(",")[0],
    latitude: body.lat ? parseFloat(body.lat) : latitude,
    longitude: body.lon ? parseFloat(body.lon) : longitude,
    admin1: address.state,
    country: address.country,
    countryCode: address.country_code?.toUpperCase(),
  };
}

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

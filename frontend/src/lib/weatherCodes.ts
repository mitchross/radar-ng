interface WeatherInfo {
  label: string;
  icon: string;
}

const WMO_CODES: Record<number, WeatherInfo> = {
  0: { label: "Clear sky", icon: "\u2600\uFE0F" },
  1: { label: "Mainly clear", icon: "\uD83C\uDF24\uFE0F" },
  2: { label: "Partly cloudy", icon: "\u26C5" },
  3: { label: "Overcast", icon: "\u2601\uFE0F" },
  45: { label: "Fog", icon: "\uD83C\uDF2B\uFE0F" },
  48: { label: "Rime fog", icon: "\uD83C\uDF2B\uFE0F" },
  51: { label: "Light drizzle", icon: "\uD83C\uDF26\uFE0F" },
  53: { label: "Drizzle", icon: "\uD83C\uDF26\uFE0F" },
  55: { label: "Dense drizzle", icon: "\uD83C\uDF26\uFE0F" },
  61: { label: "Light rain", icon: "\uD83C\uDF27\uFE0F" },
  63: { label: "Rain", icon: "\uD83C\uDF27\uFE0F" },
  65: { label: "Heavy rain", icon: "\uD83C\uDF27\uFE0F" },
  71: { label: "Light snow", icon: "\uD83C\uDF28\uFE0F" },
  73: { label: "Snow", icon: "\uD83C\uDF28\uFE0F" },
  75: { label: "Heavy snow", icon: "\uD83C\uDF28\uFE0F" },
  77: { label: "Snow grains", icon: "\uD83C\uDF28\uFE0F" },
  80: { label: "Light showers", icon: "\uD83C\uDF26\uFE0F" },
  81: { label: "Showers", icon: "\uD83C\uDF27\uFE0F" },
  82: { label: "Heavy showers", icon: "\uD83C\uDF27\uFE0F" },
  85: { label: "Light snow showers", icon: "\uD83C\uDF28\uFE0F" },
  86: { label: "Snow showers", icon: "\uD83C\uDF28\uFE0F" },
  95: { label: "Thunderstorm", icon: "\u26C8\uFE0F" },
  96: { label: "Thunderstorm + hail", icon: "\u26C8\uFE0F" },
  99: { label: "Thunderstorm + heavy hail", icon: "\u26C8\uFE0F" },
};

export function getWeatherInfo(code: number): WeatherInfo {
  return WMO_CODES[code] ?? { label: "Unknown", icon: "\u2753" };
}

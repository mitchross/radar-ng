import {
  activeLocationLabel,
  activeLocationName,
  formatPlaceLabel,
} from "../../src/lib/locationLabel";
import type { SelectedPlace } from "../../src/types/location";

const grandRapids: SelectedPlace = {
  id: 4994358,
  name: "Grand Rapids",
  admin1: "Michigan",
  country: "United States",
  latitude: 42.9634,
  longitude: -85.6681,
};

describe("location labels", () => {
  it("keeps the full place label for settings and accessibility", () => {
    expect(formatPlaceLabel(grandRapids)).toBe("Grand Rapids, Michigan");
    expect(activeLocationLabel("city", grandRapids, null)).toBe(
      "Grand Rapids, Michigan",
    );
  });

  it("uses the compact city name for editorial screen headers", () => {
    expect(activeLocationName("city", grandRapids, null)).toBe("Grand Rapids");
    expect(activeLocationName("device", null, grandRapids)).toBe("Grand Rapids");
    expect(activeLocationName("device", null, null)).toBe("My Location");
  });
});

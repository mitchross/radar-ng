import { displayTemperature } from "../../src/lib/temperature";

test.each([[32, 0], [212, 100], [-40, -40], [73, 23]])(
  "%s Fahrenheit displays as %s Celsius",
  (fahrenheit, celsius) => {
    expect(displayTemperature(fahrenheit, "celsius")).toBe(celsius);
    expect(displayTemperature(fahrenheit, "fahrenheit")).toBe(fahrenheit);
  },
);

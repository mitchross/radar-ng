import {
  CONDITION_GRADIENTS,
  CONDITION_GRADIENTS_DARK,
  CONDITION_HEADLINES,
  conditionGradient,
  type CumulusCondition,
} from "../../src/lib/cumulusTheme";

const conditions = Object.keys(CONDITION_GRADIENTS) as CumulusCondition[];

describe("condition gradients", () => {
  it.each(conditions)("%s is a real gradient in light and dark, not five identical stops", (condition) => {
    for (const dark of [false, true]) {
      const stops = conditionGradient(condition, dark);
      expect(stops).toHaveLength(5);
      expect(new Set(stops).size).toBeGreaterThan(1);
    }
    expect(conditionGradient(condition, false)).toBe(CONDITION_GRADIENTS[condition]);
    expect(conditionGradient(condition, true)).toBe(CONDITION_GRADIENTS_DARK[condition]);
  });

  it("settles into each scheme's canvas so cards keep their contrast", () => {
    for (const condition of conditions) {
      expect(CONDITION_GRADIENTS[condition][4]).toBe("#f6f2ea");
      expect(CONDITION_GRADIENTS_DARK[condition][4]).toBe("#14130f");
    }
  });

  it("has a headline for every condition", () => {
    for (const condition of conditions) expect(CONDITION_HEADLINES[condition].length).toBeGreaterThan(0);
  });
});

/** Keep reading screens comfortable in full-screen iPad and resizable windows. */
export const READING_COLUMN_WIDTH = 820;

export const readingColumnStyle = {
  width: "100%" as const,
  maxWidth: READING_COLUMN_WIDTH,
  alignSelf: "center" as const,
};

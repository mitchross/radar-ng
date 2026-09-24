// Two projects: fast ts-jest unit suites in node, and render tests that mount
// real components under the jest-expo (React Native) preset.
/** @type {import('ts-jest').JestConfigWithTsJest} */
const unit = {
  displayName: "unit",
  preset: "ts-jest",
  testEnvironment: "node",
  roots: ["<rootDir>/__tests__"],
  testPathIgnorePatterns: ["/__tests__/render/"],
  moduleNameMapper: {
    "^@/(.*)$": "<rootDir>/src/$1",
    "^expo-constants$": "<rootDir>/test-mocks/expo-constants.ts",
    "^react-native$": "<rootDir>/test-mocks/react-native.ts",
  },
  transform: {
    "^.+\\.tsx?$": [
      "ts-jest",
      {
        tsconfig: {
          strict: true,
          esModuleInterop: true,
          allowSyntheticDefaultImports: true,
        },
      },
    ],
  },
};

/** @type {import('jest').Config} */
const render = {
  displayName: "render",
  preset: "jest-expo/ios",
  roots: ["<rootDir>/__tests__/render"],
  setupFilesAfterEnv: ["<rootDir>/__tests__/render/setup.tsx"],
  // babel-preset-expo for tests only; the app has no babel.config.js and
  // Metro applies the same preset by default.
  transform: {
    "\\.[jt]sx?$": [
      "babel-jest",
      { presets: ["babel-preset-expo"], caller: { name: "metro", bundler: "metro", platform: "ios" } },
    ],
  },
  testMatch: ["**/*.test.tsx"],
  moduleNameMapper: {
    "^@/(.*)$": "<rootDir>/src/$1",
  },
};

module.exports = { projects: [unit, render] };

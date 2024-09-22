/** @type {import('ts-jest').JestConfigWithTsJest} */
module.exports = {
  preset: "ts-jest",
  testEnvironment: "node",
  setupFiles: ["<rootDir>/tests/setup.js"],
  moduleNameMapper: {
    "^vscode$": "<rootDir>/tests/__mocks__/vscode",
  },
};

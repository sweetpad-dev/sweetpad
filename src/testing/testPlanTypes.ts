
export interface Configuration {
  id: string;
  name: string;
  options: Record<string, unknown>;
}

export interface EnvironmentVariableEntry {
  key: string;
  value: string;
}

export interface LocationScenario {
  identifier: string;
  referenceType: string;
}

export interface TargetForVariableExpansion {
  containerPath: string;
  identifier: string;
  name: string;
}

export interface DefaultOptions {
  codeCoverage: boolean;
  environmentVariableEntries: EnvironmentVariableEntry[];
  language: string;
  locationScenario: LocationScenario;
  region: string;
  targetForVariableExpansion: TargetForVariableExpansion;
}

export interface Target {
  containerPath: string;
  identifier: string;
  name: string;
}

export interface TestTarget {
  parallelizable?: boolean;
  skippedTests?: string[];
  target: Target;
}

export interface TestPlan {
  configurations: Configuration[];
  defaultOptions: DefaultOptions;
  testTargets: TestTarget[];
  version: number;
}
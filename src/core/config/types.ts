export type ConfigSchema = {
  "format.path": string;
  "format.args": string[] | null;
  "format.selectionArgs": string[] | null;
  "build.xcbeautifyEnabled": boolean;
  "build.xcodeWorkspacePath": string;
  "build.xcodebuildCommand": string;
  "build.swiftCommand": string;
  "build.derivedDataPath": string;
  "build.configuration": string;
  "build.schemes.include": string[];
  "build.schemes.exclude": string[];
  "build.arch": "x86_64" | "arm64";
  "build.allowProvisioningUpdates": boolean;
  "build.args": string[];
  "build.env": { [key: string]: string | null };
  "build.launchArgs": string[];
  "build.launchEnv": { [key: string]: string };
  "build.rosettaDestination": boolean;
  "build.bringSimulatorToForeground": boolean;
  "build.autoRefreshSchemes": boolean;
  "build.autoRefreshSchemesDelay": number;
  "build.autoGenerateBuildServerConfig": boolean;
  "build.autoRestartSwiftLSP": boolean;
  "build.logStreamEnabled": boolean;
  "build.logStreamPredicate": string;
  "build.deviceTunnelAutoStart": boolean;
  "build.pymobiledevice3Path": string;
  "build.pymobiledevice3ExtraArgs": (string | null)[];
  "build.pymobiledevice3SubsystemDenyList": string[];
  "build.pymobiledevice3SubsystemAllowList": string[];
  "system.taskExecutor": "v2" | "v3";
  "system.logLevel": "debug" | "info" | "warn" | "error";
  "shellEnv.timeout": number;
  "shellEnv.shell": string | null;
  "system.enableSentry": boolean;
  "system.autoRevealTerminal": boolean;
  "system.showProgressStatusBar": boolean;
  "system.customXcodeWorkspaceParser": boolean;
  "xcodegen.autogenerate": boolean;
  "xcodebuildserver.autogenerate": boolean;
  "xcodebuildserver.path": string;
  "xcodebuildserver.serverEnv": { [key: string]: string | null };
  "tuist.autogenerate": boolean;
  "tuist.generate.env": { [key: string]: string | null };
  "testing.configuration": string;
};

export type ConfigKey = keyof ConfigSchema;

export interface ConfigProvider {
  get<K extends ConfigKey>(key: K): ConfigSchema[K] | undefined;
  isDefined<K extends ConfigKey>(key: K): boolean;
  update<K extends ConfigKey>(key: K, value: ConfigSchema[K]): Promise<void>;
}

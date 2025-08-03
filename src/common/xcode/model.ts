/**
 * Represents a build configuration (e.g., Debug, Release) with its custom settings
 */
export class XcodeConfiguration {
  public readonly name: string;
  public readonly settings: { [key: string]: string };

  constructor(options: {
    name: string;
    settings?: { [key: string]: string };
  }) {
    this.name = options.name;
    this.settings = options.settings ?? {};
  }
}

/**
 * Defines a build target (e.g., an app, framework, or test bundle) within a project
 * Each target has its own settings and references available configurations
 */
export class XcodeTarget {
  public readonly name: string;
  public readonly project: XcodeProject;
  public readonly configurations: XcodeConfiguration[];
  public readonly settings: { [key: string]: string };

  constructor(options: {
    name: string;
    project: XcodeProject;
    configurations: XcodeConfiguration[];
    settings?: { [key: string]: string };
  }) {
    this.name = options.name;
    this.project = options.project;
    this.configurations = options.configurations;
    this.settings = options.settings ?? {};
  }
}

/**
 * Models a single Xcode project containing targets, configurations, and schemes
 */
export class XcodeProject {
  public readonly name: string;
  public readonly workspace: XcodeWorkspace;
  public readonly targets: XcodeTarget[];
  public readonly configurations: XcodeConfiguration[];
  public readonly schemes: XcodeScheme[];

  constructor(options: {
    name: string;
    workspace: XcodeWorkspace;
    targets?: XcodeTarget[];
    configurations?: XcodeConfiguration[];
    schemes?: XcodeScheme[];
  }) {
    this.name = options.name;
    this.workspace = options.workspace;
    this.targets = options.targets ?? [];
    this.configurations = options.configurations ?? [];
    this.schemes = options.schemes ?? [];
  }
}

/**
 * Represents a scheme, defining how to build, run, test, and archive targets
 * Schemes can span multiple projects within a workspace
 */
export class XcodeScheme {
  public readonly name: string;
  public readonly workspace: XcodeWorkspace;
  public readonly buildTargets: { target: XcodeTarget; configuration: XcodeConfiguration }[];
  public readonly runAction: {
    target: XcodeTarget;
    configuration: XcodeConfiguration;
    arguments?: string[];
    environment?: { [key: string]: string };
  };
  public readonly testAction?: { target: XcodeTarget; configuration: XcodeConfiguration };
  public readonly archiveAction?: { target: XcodeTarget; configuration: XcodeConfiguration };

  constructor(options: {
    name: string;
    workspace: XcodeWorkspace;
    buildTargets: { target: XcodeTarget; configuration: XcodeConfiguration }[];
    runAction: {
      target: XcodeTarget;
      configuration: XcodeConfiguration;
      arguments?: string[];
      environment?: { [key: string]: string };
    };
    testAction?: { target: XcodeTarget; configuration: XcodeConfiguration };
    archiveAction?: { target: XcodeTarget; configuration: XcodeConfiguration };
  }) {
    this.name = options.name;
    this.workspace = options.workspace;
    this.buildTargets = options.buildTargets;
    this.runAction = options.runAction;
    this.testAction = options.testAction;
    this.archiveAction = options.archiveAction;
  }
}

/**
 * Top-level container grouping multiple projects and shared schemes
 */
export class XcodeWorkspace {
  // Path to the .xcworkspace directory
  public readonly path: string;
  public readonly _projects: XcodeProject[] | undefined;
  public readonly _schemes: XcodeScheme[] | undefined;

  constructor(options: {
    path: string;
  }) {
    this.path = options.path;
  }

  async getProjects(): Promise<XcodeProject[]> {
    if (this._projects) {
      return this._projects;
    }
    return [];
  }

  async getSchemes(): Promise<XcodeScheme[]> {
    if (this._schemes) {
      return this._schemes;
    }
    return [];
  }
}

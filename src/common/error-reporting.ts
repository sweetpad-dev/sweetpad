import { AsyncLocalStorage } from "node:async_hooks";
import * as Sentry from "@sentry/node";
import type * as vscode from "vscode";
import { getWorkspaceConfig } from "./config";
import { commonLogger } from "./logger";

// Injected by esbuild at build time.
declare const GLOBAL_SENTRY_DSN: string | undefined;
declare const GLOBAL_RELEASE_VERSION: string | undefined;




/**
 * Error reporting class that is responsible for capturing and sending errors to Sentry.
 * 
 * Avoid using this class directly, instead use the `errorReporting` instance.
 */
class SentryErrorReporting {
    private dsn: string | undefined;
    private isEnabled: boolean;
    private client: Sentry.NodeClient;
    private globalScope: Sentry.Scope;

    private asyncScopeStorage = new AsyncLocalStorage<Sentry.Scope>();

    constructor() {
        // By default error reporting is disabled. To enable it, set the `system.enableSentry` 
        // to `true` in the workspace settings.
        this.isEnabled = getWorkspaceConfig("system.enableSentry") ?? false;

        // This variable should be injected by esbuild at build time.
        this.dsn = GLOBAL_SENTRY_DSN;
        this.client = new Sentry.NodeClient({
            dsn: this.dsn,
            enabled: this.isEnabled,
            tracesSampleRate: 1.0,// 100% of traces will be sent to Sentry
            stackParser: Sentry.defaultStackParser,
            transport: Sentry.makeNodeTransport,
            release: GLOBAL_RELEASE_VERSION,
            integrations: [],
        })
        this.globalScope = new Sentry.Scope();
        this.globalScope.setClient(this.client);
    }

    logSetup() {
        commonLogger.log("Sentry setup", {
            sentryDsn: this.dsn ?? "<not set>",
            sentryIsEnabled: this.isEnabled,
            sentryIsClientIsInitialized: !!this.client,
        });
    }

    get currentScope() {
        return this.asyncScopeStorage.getStore() ?? this.globalScope;
    }

    async captureException(error: unknown): Promise<void> {
        this.currentScope.captureException(error);
        this.client.flush();
    }

    async addBreadcrumb(breadcrumb: Sentry.Breadcrumb): Promise<void> {
        this.currentScope.addBreadcrumb(breadcrumb);
    }


    withScope<T>(callback: (scope: Sentry.Scope) => T): T {
        const scope = this.currentScope.clone();
        return this.asyncScopeStorage.run(scope, () => callback(scope));
    }
}

export const errorReporting = new SentryErrorReporting();


export function addTreeProviderErrorReporting<T extends vscode.TreeItem>(treeDataProvider: vscode.TreeDataProvider<T>) {
    const originalGetChildren = treeDataProvider.getChildren.bind(treeDataProvider);
    const originalGetTreeItem = treeDataProvider.getTreeItem.bind(treeDataProvider);
    treeDataProvider.getChildren = async (element?: T) => {
        try {
            return await originalGetChildren(element);
        } catch (error) {
            await errorReporting.captureException(error);
            throw error;
        }
    }
    treeDataProvider.getTreeItem = async (element: T) => {
        try {
            return await originalGetTreeItem(element);
        } catch (error) {
            await errorReporting.captureException(error);
            throw error;
        }
    }
    return treeDataProvider;

}
declare module "execa" {
  export interface ExecaReturnValue {
    stdout: string;
    stderr: string;
    failed: boolean;
    timedOut: boolean;
    killed: boolean;
    signal?: string;
    signalDescription?: string;
    cwd: string;
    message: string;
    shortMessage: string;
    originalMessage: string;
    exitCode: number;
  }

  export function execa(
    command: string,
    args: string[],
    options: {
      cwd: string;
      buffer?: boolean;
      env?: { [key: string]: string | undefined };
    },
  ): Promise<ExecaReturnValue>;
}

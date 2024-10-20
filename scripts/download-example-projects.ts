import { existsSync } from "node:fs";
import { join } from "node:path";
import { execa } from "execa";

interface Repository {
  url: string;
  hash: string;
  folder: string;
}

const repos: Repository[] = [
  {
    url: "git@github.com:Brett-Best/Ampol.git",
    hash: "cfe4bb5ee814ee555e8038d2b119c656d13d0816",
    folder: "tests/examples/Ampol",
  },
  {
    url: "git@github.com:TribeMedia/meteor-ios.git",
    hash: "affe8d13a876a0e6e7c95656339d496cbc54a649",
    folder: "tests/examples/meteor-ios",
  },
  {
    url: "git@github.com:sweetpad-dev/sweetpad-demo-cocoapods.git",
    hash: "f2cd4200fd0c93287eab978a8696d450bbc37dd7",
    folder: "tests/examples/sweetpad-demo-cocoapods",
  },
  {
    url: "git@github.com:sweetpad-dev/sweetpad-demo-xcodegen.git",
    hash: "6aba6f0063fb22cdf9764950d96f911b8b953125",
    folder: "tests/examples/sweetpad-demo-xcodegen",
  },
  {
    url: "https://github.com/sweetpad-dev/sweetpad-multiproject.git",
    hash: "dea36db3affd000e3893bb9f14805cb043565d09",
    folder: "tests/examples/sweetpad-multiproject",
  },
  {
    url: "git@github.com:karona-srun/take_notes.git",
    hash: "b61e653e7332530858eddc9e5ac3a4c269160cb6",
    folder: "tests/examples/take_notes",
  },
  {
    url: "git@github.com:hyzyla/terminal23.git",
    hash: "4b59488af3766fd8735e15c6bf2e17f75707514d",
    folder: "tests/examples/terminal23",
  },
];

// Helper function to run shell commands using an array of arguments
async function runCommand(command: string, args: string[], options = {}): Promise<void> {
  console.log(`Running command: ${command} ${args.join(" ")}`);
  await execa(command, args, { stdio: "inherit", ...options });
}

async function downloadRepos(): Promise<void> {
  for (const repo of repos) {
    const { url, hash, folder } = repo;
    if (existsSync(folder)) {
      console.log(`Folder ${folder} already exists. Skipping...`);
      continue;
    }

    const folderPath = join(process.cwd(), folder);
    await runCommand("git", ["clone", url, folderPath]);
    await runCommand("git", ["checkout", hash], { cwd: folderPath });

    console.log(`Successfully cloned ${url} into ${folder} and checked out ${hash}`);
  }
}

async function main() {
  try {
    await downloadRepos();
    console.log("All repositories downloaded and moved successfully");
  } catch (error) {
    console.error("Error during the download process:", error);
  }
}

void main();

import type * as React from "react";
import Layout from "@theme/Layout";
import styles from "./index.module.css";
import Link from "@docusaurus/Link";

const toneClass: Record<string, string> = {
	ok: styles.tOk,
	err: styles.tErr,
	dim: styles.tDim,
};

/** One line of a terminal transcript. `tone` colors it the way a terminal would. */
function TLine(props: { tone?: "ok" | "err" | "dim"; children: React.ReactNode }) {
	const tone = props.tone ? ` ${toneClass[props.tone]}` : "";
	return <div className={`${styles.tLine}${tone}`}>{props.children}</div>;
}

function HeroBanner() {
	return (
		<div className={styles.hero} data-theme="dark">
			<div className={styles.heroTextAndImage}>
				<div className={styles.heroTextAndButtons}>
					<span className={styles.heroText}>
						Build, run, and test <b>Xcode</b> apps without opening <b>Xcode</b>
					</span>
					<p className={styles.heroSubtext}>
						One command builds your app, launches it on a simulator or a real
						device, and streams the logs into your terminal.{" "}
						<b>sweetpad</b> drives Xcode's own build tools underneath, so it's
						the same build with output a person can read.
					</p>
					<div className={styles.terminal}>
						<div className={styles.terminalLine}>
							<span className={styles.prompt}>$</span> brew install
							sweetpad-dev/tap/sweetpad
						</div>
						<div className={styles.terminalLine}>
							<span className={styles.prompt}>$</span> sweetpad run --on
							&quot;iPhone 16 Pro&quot;
						</div>
					</div>
					<div className={styles.heroButtons}>
						<Link
							className="button button--primary button--lg"
							to="/docs/cli/getting-started"
						>
							Run your first build
						</Link>
						<Link
							className="button button--secondary button--lg"
							to="/docs/cli/reference"
						>
							Command reference
						</Link>
					</div>
				</div>
				<img
					className={styles.heroImage}
					src="/images/logo.png"
					alt="SweetPad logo"
				/>
			</div>
		</div>
	);
}

/**
 * Numbers a reader can check, in the place they'd look for a reason to keep
 * scrolling. Rounded down so they stay true between edits.
 */
function StatsBar() {
	return (
		<section className={styles.stats} data-theme="dark">
			<div className={styles.statsInner}>
				<Link className={styles.stat} href="https://github.com/sweetpad-dev/sweetpad">
					<span className={styles.statNumber}>1,800+</span>
					<span className={styles.statLabel}>stars on GitHub</span>
				</Link>
				<Link
					className={styles.stat}
					href="https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad"
				>
					<span className={styles.statNumber}>61,000+</span>
					<span className={styles.statLabel}>VS Code extension installs</span>
				</Link>
				<div className={styles.stat}>
					<span className={styles.statNumber}>5</span>
					<span className={styles.statLabel}>
						Apple platforms: iOS, macOS, tvOS, watchOS, visionOS
					</span>
				</div>
			</div>
		</section>
	);
}

/**
 * What the tagline means, shown rather than claimed: the same build through both
 * tools. Both transcripts are real captures of a two-file SwiftUI app, trimmed
 * only where the fade says so.
 */
function ProofBand() {
	return (
		<section className={styles.proof} data-theme="dark">
			<div className={styles.proofInner}>
				<h2 className={styles.proofHeading}>xcodebuild for humans</h2>
				<p className={styles.proofLead}>
					Same compiler, same build system, same app. What changes is what lands
					in your terminal: seven lines instead of 350, on the same build.
				</p>

				<div className={styles.proofGrid}>
					<div className={styles.panel}>
						<div className={styles.panelHead}>
							<span>xcodebuild</span>
							<span className={styles.panelBadgeMuted}>350 lines</span>
						</div>
						<div className={styles.panelBodyWrap}>
							<div className={`${styles.panelBody} ${styles.panelBodyClipped}`}>
								<TLine tone="dim">Command line invocation:</TLine>
								<TLine tone="dim">
									{"    "}/Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild
									-project MyApp.xcodeproj -scheme MyApp -destination …
								</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">
									ComputePackagePrebuildTargetDependencyGraph
								</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">Prepare packages</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">CreateBuildRequest</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">SendProjectDescription</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">CreateBuildOperation</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">ComputeTargetDependencyGraph</TLine>
								<TLine tone="dim">note: Building targets in dependency order</TLine>
								<TLine tone="dim">
									note: Target dependency graph (1 target)
								</TLine>
								<TLine tone="dim">
									{"    "}Target &apos;MyApp&apos; in project &apos;MyApp&apos;
									(no dependencies)
								</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">GatherProvisioningInputs</TLine>
								<TLine>&nbsp;</TLine>
								<TLine tone="dim">CreateBuildDescription</TLine>
							</div>
							<div className={styles.panelFade} />
						</div>
					</div>

					<div className={styles.panel}>
						<div className={styles.panelHead}>
							<span>sweetpad</span>
							<span className={styles.panelBadge}>7 lines</span>
						</div>
						<div className={styles.panelBodyWrap}>
							<div className={styles.panelBody}>
								<TLine>
									building MyApp (Debug) for platform=iOS
									Simulator,id=F92801F8-…
								</TLine>
								<TLine>{"  "}Linking __preview.dylib</TLine>
								<TLine>{"  "}Compiling ContentView.swift</TLine>
								<TLine>{"  "}Compiling MyApp.swift</TLine>
								<TLine>{"  "}Linking MyApp.debug.dylib</TLine>
								<TLine>{"  "}Linking MyApp</TLine>
								<TLine tone="ok">✓ Build succeeded (6.7s)</TLine>
							</div>
						</div>
					</div>
				</div>

				<p className={styles.proofLead}>
					And when it breaks, the error is the third line instead of the
					forty-fourth.
				</p>

				<div className={`${styles.panel} ${styles.panelWide}`}>
					<div className={styles.panelBodyWrap}>
						<div className={styles.panelBody}>
							<TLine>
								<span className={styles.prompt}>$</span> sweetpad build
							</TLine>
							<TLine>
								building MyApp (Debug) for platform=iOS Simulator,id=F92801F8-…
							</TLine>
							<TLine>{"  "}Compiling ContentView.swift</TLine>
							<TLine tone="err">
								error: Sources/App/ContentView.swift:12:14: cannot find
								&apos;greeting&apos; in scope
							</TLine>
							<TLine tone="err">✗ Build failed</TLine>
							<TLine tone="dim">error: building the project</TLine>
							<TLine tone="dim">
								{"  "}xcodebuild exited with a non-zero status
							</TLine>
						</div>
					</div>
				</div>

				<p className={styles.proofFoot}>
					Works with Xcode projects and workspaces, Tuist, XcodeGen, and Swift
					Packages.
				</p>
			</div>
		</section>
	);
}

function CliFeatures() {
	return (
		<section className={styles.features}>
			<div className="container">
				<h2 className={styles.sectionHeading}>Inside the CLI</h2>
				<div className="row">
					<FeatureItem
						title="🛠️ Build & run"
						description="One command builds the app, launches it, and streams its logs into your terminal. Press r to rebuild without leaving."
						link="/docs/cli/build-and-run"
					/>
					<FeatureItem
						title="🔥 Hot reload"
						description="Save a Swift file and the running app updates in place, keeping its current screen and state. No rebuild, no relaunch."
						link="/docs/cli/hot-reload"
					/>
					<FeatureItem
						title="🧠 Autocomplete anywhere"
						description="One command wires SourceKit-LSP to SweetPad's build server, so completions work in Neovim, Zed, Helix, or Emacs."
						link="/docs/cli/autocomplete"
					/>
				</div>
				<div className="row">
					<FeatureItem
						title="✅ Testing"
						description="Run the suite, narrow it to one test, watch for saves, and pull the screenshots a failing UI test recorded."
						link="/docs/cli/testing"
					/>
					<FeatureItem
						title="📱 Simulators & devices"
						description="Run on a simulator by name, on a connected iPhone, or on your Mac. SweetPad remembers the choice, so you pick once."
						link="/docs/cli/destinations"
					/>
					<FeatureItem
						title="🐛 Debug & diagnose"
						description="Follow logs until a line matches, script an lldb session, or catch the first crash and get a structured report."
						link="/docs/cli/app-lifecycle"
					/>
				</div>
				<div className="row">
					<FeatureItem
						title="🤖 Scripts & CI"
						description="Every command speaks JSON and returns meaningful exit codes, so it drops into git hooks and pipelines."
						link="/docs/cli/scripts-and-ci"
					/>
					<FeatureItem
						title="🧩 AI agent skills"
						description="Vendor-neutral skill files that teach a coding agent to drive the CLI instead of guessing at xcodebuild."
						link="/docs/cli/agent-skills"
					/>
					<FeatureItem
						title="📖 Every command"
						description="Commands, flags, config keys, and exit codes. The whole surface on one page."
						link="/docs/cli/reference"
					/>
				</div>
				<p className={styles.sectionNote}>
					Also: <Link to="/docs/cli/project">Swift Package dependencies</Link>,{" "}
					<Link to="/docs/cli/archive">archiving and export</Link>,{" "}
					<Link to="/docs/cli/generated-projects">Tuist and XcodeGen</Link>, and{" "}
					<Link to="/docs/cli/merge">git merge drivers</Link> that resolve{" "}
					<code>.pbxproj</code> conflicts semantically.
				</p>
			</div>
		</section>
	);
}

function FeatureItem(props: {
	title: string;
	description: string;
	link: string;
}) {
	return (
		<div className="col col--4">
			<Link className={styles.featureLink} to={props.link}>
				{props.title}
			</Link>
			<p>{props.description}</p>
		</div>
	);
}

function VscodeBand() {
	return (
		<section className={styles.band}>
			<div className="container">
				<div className={styles.bandInner}>
					<div>
						<h2 className={styles.bandHeading}>
							🧩 Prefer to work in VS Code?
						</h2>
						<p className={styles.bandText}>
							The SweetPad <b>extension</b> does the same builds, runs, and tests
							from the VS Code sidebar, and adds breakpoints, a native Testing
							panel, format-on-save, and autocomplete. It works in Cursor too.
						</p>
						<p className={styles.bandNote}>
							It's a separate install, and it doesn't need the CLI.{" "}
							<Link to="/docs">Which one do I need?</Link>
						</p>
						<div className={styles.bandLinks}>
							<Link
								className="button button--primary"
								to="/docs/vscode/getting-started"
							>
								Extension docs
							</Link>
							<Link
								className="button button--secondary"
								href="https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad"
							>
								VS Code Marketplace
							</Link>
						</div>
					</div>
					<ul className={styles.bandList}>
						<li>
							<Link to="/docs/vscode/build">Build & Run from the sidebar</Link>
						</li>
						<li>
							<Link to="/docs/vscode/debug">Debug with breakpoints</Link>
						</li>
						<li>
							<Link to="/docs/vscode/tests">Tests in the Testing panel</Link>
						</li>
						<li>
							<Link to="/docs/vscode/autocomplete">
								Autocomplete via SourceKit-LSP
							</Link>
						</li>
						<li>
							<Link to="/docs/vscode/format">Format on save</Link>
						</li>
					</ul>
				</div>
			</div>
		</section>
	);
}

/**
 * The last thing on the page is the ask, aimed at the reader's own project
 * rather than a demo, with the cost of trying it stated plainly.
 */
function Closer() {
	return (
		<section className={styles.closer} data-theme="dark">
			<div className={styles.closerInner}>
				<h2 className={styles.closerHeading}>
					Try it on a project you already have
				</h2>
				<p className={styles.closerText}>
					Install it, change into any Xcode project, and run one command. A Mac
					with Xcode is the whole dependency list.
				</p>
				<div className={styles.terminal}>
					<div className={styles.terminalLine}>
						<span className={styles.prompt}>$</span> brew install
						sweetpad-dev/tap/sweetpad
					</div>
					<div className={styles.terminalLine}>
						<span className={styles.prompt}>$</span> cd MyApp &amp;&amp; sweetpad
						run
					</div>
				</div>
				<div className={styles.closerButtons}>
					<Link
						className="button button--primary button--lg"
						to="/docs/cli/getting-started"
					>
						Run your first build
					</Link>
					<Link
						className="button button--secondary button--lg"
						href="https://github.com/sweetpad-dev/sweetpad"
					>
						Star on GitHub
					</Link>
				</div>
				<p className={styles.closerNote}>
					Free and open source under the MIT license. SweetPad writes nothing
					into your project unless you ask it to, and{" "}
					<code>brew uninstall sweetpad</code> takes it all back out.
				</p>
			</div>
		</section>
	);
}

export default function Home(): React.JSX.Element {
	return (
		<Layout
			title={"Home"}
			description="xcodebuild for humans. SweetPad builds, runs, debugs, and tests Xcode apps from the terminal, or from the VS Code sidebar with the extension."
		>
			<main>
				<HeroBanner />
				<StatsBar />
				<CliFeatures />
				<ProofBand />
				<VscodeBand />
				<Closer />
			</main>
		</Layout>
	);
}

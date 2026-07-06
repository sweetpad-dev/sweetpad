import type * as React from "react";
import Layout from "@theme/Layout";
import styles from "./index.module.css";
import Link from "@docusaurus/Link";

function HeroBanner() {
	return (
		<div className={styles.hero} data-theme="dark">
			<div className={styles.heroTextAndImage}>
				<div className={styles.heroTextAndButtons}>
					<span className={styles.heroText}>
						A suite of tools to work with <b>Xcode</b> projects — from{" "}
						<b>VS Code</b> and the <b>terminal</b>
					</span>
					<div className={styles.heroButtons}>
						<Link
							className="button button--primary button--lg"
							to="/docs/getting-started-vscode"
						>
							VS Code extension
						</Link>
						<Link
							className="button button--secondary button--lg"
							to="/docs/getting-started-cli"
						>
							Command-line tool
						</Link>
					</div>
				</div>
				<img className={styles.heroImage} src="/images/logo.png" alt="Hero" />
			</div>
		</div>
	);
}

function Products() {
	return (
		<section className={styles.features}>
			<div className="container">
				<p style={{ textAlign: "center", maxWidth: 760, margin: "0 auto 2rem" }}>
					SweetPad is a suite of tools for working with Xcode projects. It comes
					in two forms — pick whichever fits how you work:
				</p>
				<div className="row">
					<FeatureItem
						title="🧩 VS Code extension"
						description="Build, run, debug, and test from the editor sidebar. Works in Cursor too."
						link="/docs/getting-started-vscode"
					/>
					<FeatureItem
						title="💻 Command-line tool"
						description="The standalone sweetpad CLI: build, run, and test from the terminal — no editor required."
						link="/docs/getting-started-cli"
					/>
				</div>
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

function Features() {
	return (
		<section className={styles.features}>
			<div className="container">
				<h2 style={{ textAlign: "center", marginBottom: "2rem" }}>
					Inside the VS Code extension
				</h2>
				<div className="row">
					<FeatureItem
						title="✅ Autocomplete"
						description="Setup autocomplete using xcode-build-server"
						link="/docs/autocomplete"
					/>
					<FeatureItem
						title="🛠️ Build & Run"
						description="Build and run application using xcodebuild"
						link="/docs/build"
					/>
					<FeatureItem
						title="💅🏼 Format"
						description="Format files using swift-format or other formatter of your choice"
						link="/docs/format"
					/>
				</div>
				<div className="row">
					<FeatureItem
						title="📱 Simulator"
						description="Manage iOS simulators"
						link="/docs/simulators"
					/>
					<FeatureItem
						title="📱 Devices"
						description="Run iOS applications on iPhone or iPad"
						link="/docs/devices"
					/>
					<FeatureItem
						title="🛠️ Tools"
						description="Manage essential iOS development tools using Homebrew"
						link="/docs/tools"
					/>
				</div>
				<div className="row">
					<FeatureItem
						title="🪲 Debug"
						description="Debug iOS applications using CodeLLDB"
						link="/docs/debug"
					/>
					<FeatureItem
						title="🔎 Tests"
						description="Run tests on simulators and devices"
						link="/docs/tests"
					/>
				</div>
			</div>
		</section>
	);
}

export default function Home(): React.JSX.Element {
	return (
		<Layout
			title={"Home"}
			description="Description will go into a meta tag in <head />"
		>
			<main>
				<HeroBanner />
				<Products />
				<Features />
			</main>
		</Layout>
	);
}

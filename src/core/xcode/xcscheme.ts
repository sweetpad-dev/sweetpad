// ============================================================================
//  xcscheme.ts — typed parser/serializer for Xcode `.xcscheme` files.
//
//  ─── What is a .xcscheme? ──────────────────────────────────────────────────
//
//  An .xcscheme is the XML file Xcode reads/writes from its scheme editor
//  (Product → Edit Scheme…). One scheme describes how a target gets built and
//  exercised across six "actions":
//
//    Build     — which targets are compiled, and for which downstream actions.
//    Test      — which test bundles run, plus sanitizers / code coverage /
//                test plans / args / env.
//    Run       — the app/process Xcode launches (in this file: <LaunchAction>,
//                which Xcode's UI labels "Run"). Owns args, env, App Language,
//                App Region, sanitizers, GPU validation, location simulation.
//    Profile   — the Instruments launch config (similar shape to Run).
//    Analyze   — clang static analyzer config (essentially just a build
//                configuration).
//    Archive   — App Store / distribution build (archive name, organizer
//                reveal).
//
//  Schemes live in either xcshareddata/xcschemes/ (shared, committed) or
//  xcuserdata/<user>.xcuserdatad/xcschemes/ (per-user, gitignored).
//
//  ─── XML format quirks Xcode emits ─────────────────────────────────────────
//
//  - 3-space indent per nesting level. NOT 2, NOT 4. Xcode's own quirk.
//  - Attributes are aligned one-per-line at parent-indent + 3 spaces, with
//    spaces around `=`, e.g.  `   buildConfiguration = "Debug"`.
//  - Elements NEVER self-close. Even an element with attributes and no
//    children opens with `>` and closes with `</Name>` on its own line.
//  - Booleans are encoded as `"YES"` / `"NO"`, NOT `"true"` / `"false"`.
//  - Trailing newline at end of file. No BOM.
//  - Element child order is NOT the same as schema-canonical order — Xcode
//    and XcodeGen happen to disagree, e.g. XcodeGen emits <Testables> before
//    <CommandLineArguments>. Source order is preserved via the slot list so
//    both round-trip identically.
//
//  ─── Design principles ("boring OO") ───────────────────────────────────────
//
//   - No Proxy, no Object.defineProperty, no decorators, no schema-driven
//     metaprogramming. Every accessor is a plain TypeScript get/set declared
//     by hand. Stepping into them with a debugger lands at the actual line of
//     code; cmd-click navigates to the definition.
//
//   - One class per XML element type. Each class owns its own attributes
//     (via explicit getters/setters), its own typed child accessors, and its
//     own static list of allowed-child element names. Reading a single class
//     top-to-bottom tells you everything it can hold.
//
//   - Storage is an insertion-ordered Map<string,string> for attributes and a
//     parallel ChildSlot[] / childMap pair for children. The Map's order IS
//     the round-trip order — no separate __attrOrder__ array to keep in sync.
//
//   - Comments, CDATA sections, and the XML declaration are preserved.
//     Comments and CDATA are first-class entries in the slot list so they
//     interleave with element children in source order. The XML declaration
//     lives on `SchemeDocument` directly.
//
//   - Unknown elements are passed through verbatim as `GenericNode`s in a
//     ChildSlot of kind "extra". Round-trip is byte-identical even for
//     element types we haven't modeled yet — useful for forward-compat as
//     Apple adds new attributes / elements in future Xcode releases.
//
//  ─── References ────────────────────────────────────────────────────────────
//
//  - Tuist's Swift XcodeProj is the most complete public model:
//    https://github.com/tuist/XcodeProj/tree/main/Sources/XcodeProj/Scheme
//  - The 14 fixtures in tests/xcscheme-data/ (with SOURCES.md attribution)
//    cover every element type this module models, plus a couple it doesn't
//    (which pass through as extras).
//  - There is no public DTD or XSD from Apple. Everything below was derived
//    by reading those fixtures + the Swift model + experimentation.
// ============================================================================

import { XmlCdata, XmlComment, XmlDeclaration, XmlElement, XmlError, parseXml } from "@rgrove/parse-xml";

/** Schema version of the parsed-document shape — bumped when the model evolves. */
export const SCHEME_DOCUMENT_VERSION = "1";

/** XML declaration `<?xml version="..." encoding="..." standalone="..."?>`. */
export interface XmlDecl {
  version: string;
  encoding?: string;
  standalone?: "yes" | "no";
}

/** An unknown / unmodeled XML element preserved verbatim for round-trip. */
export interface GenericNode {
  name: string;
  attrs: Map<string, string>;
  slots: ChildSlot[];
  children: Map<string, GenericNode[]>;
}

/** What can appear at each position in a node's children sequence. */
export type ChildSlot =
  | { kind: "element"; name: string; index: number }
  | { kind: "comment"; text: string }
  | { kind: "cdata"; text: string }
  | { kind: "extra"; node: GenericNode };

/** Thrown when the input XML cannot be parsed as a scheme. */
export class XcSchemeParseError extends Error {
  readonly line?: number;
  readonly column?: number;
  readonly sourceContext?: string;
  constructor(message: string, options: { line?: number; column?: number; sourceContext?: string } = {}) {
    super(options.sourceContext ? `${message}\n${options.sourceContext}` : message);
    this.name = "XcSchemeParseError";
    this.line = options.line;
    this.column = options.column;
    this.sourceContext = options.sourceContext;
  }
}

export abstract class SchemeNode {
  /** Names of XML child elements this node accepts as known children. */
  static readonly allowedChildren: readonly string[] = [];

  // Insertion-ordered attribute storage. Iteration order = serialization order.
  protected attrs = new Map<string, string>();

  // The sequence of children + comments + cdata + extras, in source order.
  protected slots: ChildSlot[] = [];

  // Typed child instances keyed by XML element name. Array values for child
  // names that legitimately repeat (e.g. multiple <ExecutionAction>).
  protected childMap = new Map<string, SchemeNode[]>();

  // --- attribute primitives -----------------------------------------------

  protected getString(name: string): string | undefined {
    return this.attrs.get(name);
  }

  protected setString(name: string, value: string | undefined): void {
    if (value === undefined) this.attrs.delete(name);
    else this.attrs.set(name, value);
  }

  protected getBool(name: string): boolean | undefined {
    const v = this.attrs.get(name);
    return v === undefined ? undefined : v === "YES";
  }

  protected setBool(name: string, value: boolean | undefined): void {
    if (value === undefined) this.attrs.delete(name);
    else this.attrs.set(name, value ? "YES" : "NO");
  }

  /** Raw access to all attributes (for inspection / debugging). */
  attributeEntries(): Array<[string, string]> {
    return Array.from(this.attrs.entries());
  }

  hasAttribute(name: string): boolean {
    return this.attrs.has(name);
  }

  // --- child primitives ---------------------------------------------------

  /**
   * Single typed child by element name. Returns the first instance if there
   * are multiple, or undefined if none.
   */
  protected getChild<T extends SchemeNode>(name: string): T | undefined {
    return this.childMap.get(name)?.[0] as T | undefined;
  }

  /** All typed children with the given element name, in source order. */
  protected getChildList<T extends SchemeNode>(name: string): T[] {
    const list = this.childMap.get(name);
    return list ? (list.slice() as T[]) : [];
  }

  /**
   * Replace any existing child(ren) for `name` with a single instance. If
   * `value` is undefined, remove all children with that name.
   */
  protected setChild(name: string, value: SchemeNode | undefined): void {
    if (value === undefined) {
      this.removeAllChildren(name);
      return;
    }
    this.slots = this.slots.filter((s) => !(s.kind === "element" && s.name === name));
    this.childMap.set(name, [value]);
    this.slots.push({ kind: "element", name, index: 0 });
  }

  /** Append a typed child at the end of the slot sequence. */
  protected appendChild(name: string, child: SchemeNode): void {
    const existing = this.childMap.get(name) ?? [];
    existing.push(child);
    this.childMap.set(name, existing);
    this.slots.push({ kind: "element", name, index: existing.length - 1 });
  }

  /** Insert a typed child immediately before the first slot whose name matches `beforeName`. */
  protected insertChildBefore(name: string, child: SchemeNode, beforeName: string): void {
    const existing = this.childMap.get(name) ?? [];
    existing.push(child);
    this.childMap.set(name, existing);
    const slot: ChildSlot = { kind: "element", name, index: existing.length - 1 };
    const idx = this.slots.findIndex((s) => s.kind === "element" && s.name === beforeName);
    if (idx < 0) this.slots.push(slot);
    else this.slots.splice(idx, 0, slot);
  }

  /** Remove all children with the given element name. */
  protected removeAllChildren(name: string): void {
    this.childMap.delete(name);
    this.slots = this.slots.filter((s) => !(s.kind === "element" && s.name === name));
  }

  // --- comments / CDATA / extras -----------------------------------------

  /** All comments attached to this node, in source order, with simple position hints. */
  comments(): Array<{ text: string; precededBy?: string; followedBy?: string }> {
    const out: Array<{ text: string; precededBy?: string; followedBy?: string }> = [];
    for (let i = 0; i < this.slots.length; i++) {
      const slot = this.slots[i];
      if (slot.kind !== "comment") continue;
      let precededBy: string | undefined;
      for (let j = i - 1; j >= 0; j--) {
        const s = this.slots[j];
        if (s.kind === "element") {
          precededBy = s.name;
          break;
        }
      }
      let followedBy: string | undefined;
      for (let j = i + 1; j < this.slots.length; j++) {
        const s = this.slots[j];
        if (s.kind === "element") {
          followedBy = s.name;
          break;
        }
      }
      out.push({ text: slot.text, precededBy, followedBy });
    }
    return out;
  }

  /** Append a comment at the end of the children sequence. */
  appendComment(text: string): void {
    this.slots.push({ kind: "comment", text });
  }

  /** Insert a comment immediately before the first child element with the given name. */
  addCommentBefore(elementName: string, text: string): void {
    const idx = this.slots.findIndex((s) => s.kind === "element" && s.name === elementName);
    if (idx < 0) this.slots.push({ kind: "comment", text });
    else this.slots.splice(idx, 0, { kind: "comment", text });
  }

  /** Remove comments matching the predicate (or all of them if none given). */
  removeComments(predicate?: (text: string) => boolean): void {
    this.slots = this.slots.filter((s) => s.kind !== "comment" || (predicate ? !predicate(s.text) : false));
  }

  /** Append a CDATA section at the end of the children sequence. */
  appendCdata(text: string): void {
    this.slots.push({ kind: "cdata", text });
  }

  /** Unknown / unmodeled children that survive round-trip via the extras passthrough. */
  extraChildren(): GenericNode[] {
    return this.slots.filter((s): s is { kind: "extra"; node: GenericNode } => s.kind === "extra").map((s) => s.node);
  }

  // --- internals consumed by parser + serializer --------------------------

  /** @internal — parser uses this to populate without going through coercion. */
  _hydrateAttr(name: string, raw: string): void {
    this.attrs.set(name, raw);
  }

  /** @internal — parser appends each typed child instance. */
  _appendParsedChild(name: string, child: SchemeNode): void {
    const existing = this.childMap.get(name) ?? [];
    existing.push(child);
    this.childMap.set(name, existing);
    this.slots.push({ kind: "element", name, index: existing.length - 1 });
  }

  /** @internal — parser appends a comment slot. */
  _appendParsedComment(text: string): void {
    this.slots.push({ kind: "comment", text });
  }

  /** @internal — parser appends a CDATA slot. */
  _appendParsedCdata(text: string): void {
    this.slots.push({ kind: "cdata", text });
  }

  /** @internal — parser appends an unknown element. */
  _appendParsedExtra(node: GenericNode): void {
    this.slots.push({ kind: "extra", node });
  }

  /** @internal — serializer iterates slots and looks children up by name+index. */
  _slotsForSerialize(): readonly ChildSlot[] {
    return this.slots;
  }

  /** @internal — serializer pulls a specific child instance by name+index. */
  _childAt(name: string, index: number): SchemeNode | undefined {
    return this.childMap.get(name)?.[index];
  }

  // --- clone --------------------------------------------------------------

  /** Deep clone — useful for undo/redo and immutable update patterns. */
  clone(): this {
    const Ctor = this.constructor as new () => this;
    const copy = new Ctor();
    for (const [k, v] of this.attrs) copy.attrs.set(k, v);
    for (const [name, list] of this.childMap) {
      copy.childMap.set(
        name,
        list.map((c) => c.clone()),
      );
    }
    for (const slot of this.slots) {
      if (slot.kind === "extra") copy.slots.push({ kind: "extra", node: cloneGeneric(slot.node) });
      else copy.slots.push({ ...slot });
    }
    return copy;
  }
}

function cloneGeneric(g: GenericNode): GenericNode {
  return {
    name: g.name,
    attrs: new Map(g.attrs),
    slots: g.slots.map((s) => (s.kind === "extra" ? { kind: "extra", node: cloneGeneric(s.node) } : { ...s })),
    children: new Map(Array.from(g.children, ([k, v]) => [k, v.map(cloneGeneric)])),
  };
}

/**
 * Generic container element (e.g. <CommandLineArguments>, <Testables>) — has
 * no attributes of its own, only wraps a list of similar children. Modeled as
 * a real node so its comments and extras round-trip, but hidden behind
 * convenience methods on the parent element.
 */
class ContainerNode extends SchemeNode {}

/**
 * <BuildableReference> — the cross-reference that ties a scheme to a specific
 * target inside a project or workspace. It appears wherever a scheme needs to
 * say "this target": inside BuildActionEntry, BuildableProductRunnable,
 * MacroExpansion, EnvironmentBuildable, TestableReference, etc.
 *
 * Attributes:
 *  - BuildableIdentifier:  almost always "primary". A leftover from when
 *                          Xcode could reference secondary outputs of a
 *                          target; in practice you only ever see "primary".
 *  - BlueprintIdentifier:  the UUID of the PBXNativeTarget in project.pbxproj
 *                          (24-char hex). When the project is regenerated by
 *                          XcodeGen / Tuist, this may change.
 *  - BuildableName:        the on-disk product name, e.g. "MyApp.app" or
 *                          "Tests.xctest". Includes the file extension.
 *  - BlueprintName:        the target name as shown in Xcode's UI (e.g.
 *                          "MyApp"). This is the value Sweetpad's
 *                          targetToLaunch() returns.
 *  - ReferencedContainer:  path to the containing project relative to the
 *                          scheme, prefixed with "container:". E.g.
 *                          "container:MyApp.xcodeproj" or
 *                          "container:LocalPackages/Waitlist".
 */
export class BuildableReference extends SchemeNode {
  static override readonly allowedChildren = [] as const;

  get buildableIdentifier(): string | undefined {
    return this.getString("BuildableIdentifier");
  }
  set buildableIdentifier(v: string | undefined) {
    this.setString("BuildableIdentifier", v);
  }

  get blueprintIdentifier(): string | undefined {
    return this.getString("BlueprintIdentifier");
  }
  set blueprintIdentifier(v: string | undefined) {
    this.setString("BlueprintIdentifier", v);
  }

  get buildableName(): string | undefined {
    return this.getString("BuildableName");
  }
  set buildableName(v: string | undefined) {
    this.setString("BuildableName", v);
  }

  get blueprintName(): string | undefined {
    return this.getString("BlueprintName");
  }
  set blueprintName(v: string | undefined) {
    this.setString("BlueprintName", v);
  }

  get referencedContainer(): string | undefined {
    return this.getString("ReferencedContainer");
  }
  set referencedContainer(v: string | undefined) {
    this.setString("ReferencedContainer", v);
  }
}

/**
 * <Test> — a single test entry inside <SkippedTests> or <SelectedTests>.
 * The `Identifier` is either a class name ("ArticleFetcherTests") or a
 * class/method pair with parens ("MyTests/testSomething()").
 */
export class TestItem extends SchemeNode {
  static override readonly allowedChildren = [] as const;

  get identifier(): string | undefined {
    return this.getString("Identifier");
  }
  set identifier(v: string | undefined) {
    this.setString("Identifier", v);
  }
}

/**
 * <LocationScenarioReference> — selects a GPS scenario for location
 * simulation in the Run action. Requires `LaunchAction.allowLocationSimulation`.
 *
 *  - identifier:     scenario name. For built-in scenarios this is a city
 *                    name like "London, England"; for custom scenarios a
 *                    path to a .gpx file relative to the project.
 *  - referenceType:  "0" = built-in (Xcode's preset locations),
 *                    "1" = custom (.gpx in the workspace).
 */
export class LocationScenarioReference extends SchemeNode {
  static override readonly allowedChildren = [] as const;

  get identifier(): string | undefined {
    return this.getString("identifier");
  }
  set identifier(v: string | undefined) {
    this.setString("identifier", v);
  }

  get referenceType(): string | undefined {
    return this.getString("referenceType");
  }
  set referenceType(v: string | undefined) {
    this.setString("referenceType", v);
  }
}

/**
 * <TestPlanReference> — link to an .xctestplan file. Test plans are the
 * preferred way to configure test variants in modern Xcode (replacing many
 * of the per-scheme test settings).
 *
 *  - reference:  "container:<path>" to the .xctestplan file.
 *  - default:    YES on the test plan that runs when no specific plan is
 *                requested. Exactly one plan in <TestPlans> should be default.
 *                Exposed as `isDefault` here because `default` is a
 *                TypeScript reserved word.
 */
export class TestPlanReference extends SchemeNode {
  static override readonly allowedChildren = [] as const;

  get reference(): string | undefined {
    return this.getString("reference");
  }
  set reference(v: string | undefined) {
    this.setString("reference", v);
  }

  get isDefault(): boolean | undefined {
    return this.getBool("default");
  }
  set isDefault(v: boolean | undefined) {
    this.setBool("default", v);
  }
}

/**
 * <CommandLineArgument> — a single arg passed to the launched process. Lives
 * inside a <CommandLineArguments> container on LaunchAction / TestAction /
 * ProfileAction. The `argument` value may contain spaces (Xcode passes the
 * whole string as one argv entry — to pass multiple args, use multiple
 * <CommandLineArgument> elements).
 *
 *  - argument:   the raw string. May contain build-setting substitutions
 *                like $(SRCROOT) or `-AppleLanguages (ar)`.
 *  - isEnabled:  YES = include at launch, NO = skip (the GUI checkbox state).
 */
export class CommandLineArgument extends SchemeNode {
  static override readonly allowedChildren = [] as const;

  get argument(): string | undefined {
    return this.getString("argument");
  }
  set argument(v: string | undefined) {
    this.setString("argument", v);
  }

  get isEnabled(): boolean | undefined {
    return this.getBool("isEnabled");
  }
  set isEnabled(v: boolean | undefined) {
    this.setBool("isEnabled", v);
  }
}

/**
 * <EnvironmentVariable> — single env entry set in the launched process.
 * Disabled vars are kept in the scheme but not passed at launch.
 */
export class EnvironmentVariable extends SchemeNode {
  static override readonly allowedChildren = [] as const;

  get key(): string | undefined {
    return this.getString("key");
  }
  set key(v: string | undefined) {
    this.setString("key", v);
  }

  get value(): string | undefined {
    return this.getString("value");
  }
  set value(v: string | undefined) {
    this.setString("value", v);
  }

  get isEnabled(): boolean | undefined {
    return this.getBool("isEnabled");
  }
  set isEnabled(v: boolean | undefined) {
    this.setBool("isEnabled", v);
  }
}

/**
 * <AdditionalOption> — diagnostic options under the scheme's Test or Run
 * action ("Diagnostics" tab in Xcode's UI). These are the legacy malloc /
 * zombie checkers that predate the sanitizers; the modern equivalents
 * (Address Sanitizer, UB Sanitizer, etc.) live as boolean attrs on the
 * action element itself.
 *
 *  - key:        e.g. "NSZombieEnabled", "MallocScribble", "MallocStackLogging".
 *  - value:      "YES" / "NO" (kept as a raw string to handle the rare case
 *                of non-boolean diagnostic values).
 *  - isEnabled:  the checkbox state — independent of the value.
 */
export class AdditionalOption extends SchemeNode {
  static override readonly allowedChildren = [] as const;

  get key(): string | undefined {
    return this.getString("key");
  }
  set key(v: string | undefined) {
    this.setString("key", v);
  }

  get value(): string | undefined {
    return this.getString("value");
  }
  set value(v: string | undefined) {
    this.setString("value", v);
  }

  get isEnabled(): boolean | undefined {
    return this.getBool("isEnabled");
  }
  set isEnabled(v: boolean | undefined) {
    this.setBool("isEnabled", v);
  }
}

/**
 * <BuildableProductRunnable> — wraps the target that the Run / Profile action
 * launches as a process. The wrapped <BuildableReference> identifies the
 * target; its BlueprintName is what Sweetpad's `targetToLaunch()` returns.
 *
 *  - runnableDebuggingMode:  almost always "0" (normal). Other values exist
 *                            for legacy embedded/iOS contexts but are rarely
 *                            seen in modern schemes.
 */
export class BuildableProductRunnable extends SchemeNode {
  static override readonly allowedChildren = ["BuildableReference"] as const;

  get runnableDebuggingMode(): string | undefined {
    return this.getString("runnableDebuggingMode");
  }
  set runnableDebuggingMode(v: string | undefined) {
    this.setString("runnableDebuggingMode", v);
  }

  buildableReference(): BuildableReference | undefined {
    return this.getChild<BuildableReference>("BuildableReference");
  }
  setBuildableReference(ref: BuildableReference | undefined): void {
    this.setChild("BuildableReference", ref);
  }
}

/**
 * <RemoteRunnable> — used when the executable being debugged is already
 * installed on a remote device or hosted by another process. Less common
 * than BuildableProductRunnable.
 *
 *  - runnableDebuggingMode:  "2" for remote-app debugging.
 *  - BundleIdentifier:       bundle ID of the remote app (e.g.
 *                            "com.example.myapp").
 *  - RemotePath:             on-device path to the .app bundle.
 */
export class RemoteRunnable extends SchemeNode {
  static override readonly allowedChildren = ["BuildableReference"] as const;

  get runnableDebuggingMode(): string | undefined {
    return this.getString("runnableDebuggingMode");
  }
  set runnableDebuggingMode(v: string | undefined) {
    this.setString("runnableDebuggingMode", v);
  }

  get bundleIdentifier(): string | undefined {
    return this.getString("BundleIdentifier");
  }
  set bundleIdentifier(v: string | undefined) {
    this.setString("BundleIdentifier", v);
  }

  get remotePath(): string | undefined {
    return this.getString("RemotePath");
  }
  set remotePath(v: string | undefined) {
    this.setString("RemotePath", v);
  }

  buildableReference(): BuildableReference | undefined {
    return this.getChild<BuildableReference>("BuildableReference");
  }
  setBuildableReference(ref: BuildableReference | undefined): void {
    this.setChild("BuildableReference", ref);
  }
}

/**
 * <MacroExpansion> — Xcode's way of saying "I don't have anything runnable
 * here, but use this target's build settings to expand macros like
 * $(SRCROOT) / $(PROJECT_DIR) / $(BUILT_PRODUCTS_DIR)".
 *
 * You see this in:
 *  - LaunchAction / ProfileAction / TestAction of a framework or library
 *    scheme (no executable to launch — but scripts still need build settings)
 *  - Some app schemes where Test/Profile reuse settings from another target
 */
export class MacroExpansion extends SchemeNode {
  static override readonly allowedChildren = ["BuildableReference"] as const;

  buildableReference(): BuildableReference | undefined {
    return this.getChild<BuildableReference>("BuildableReference");
  }
  setBuildableReference(ref: BuildableReference | undefined): void {
    this.setChild("BuildableReference", ref);
  }
}

/**
 * <EnvironmentBuildable> — appears inside an <ActionContent> shell-script
 * pre/post action. Tells Xcode which target's environment / build settings to
 * use when expanding macros referenced in the script body (e.g.
 * `cd "${PROJECT_DIR}"`). Without it, the script runs without target-specific
 * substitutions.
 */
export class EnvironmentBuildable extends SchemeNode {
  static override readonly allowedChildren = ["BuildableReference"] as const;

  buildableReference(): BuildableReference | undefined {
    return this.getChild<BuildableReference>("BuildableReference");
  }
  setBuildableReference(ref: BuildableReference | undefined): void {
    this.setChild("BuildableReference", ref);
  }
}

/**
 * <ActionContent> — the body of an <ExecutionAction>. For shell-script
 * actions, this holds the script text and shell to invoke.
 *
 *  - title:           human label shown in Xcode's UI.
 *  - scriptText:      the shell script body. Multi-line is XML-entity-encoded
 *                     in the source (`&#10;` for newlines, `&quot;` for `"`)
 *                     and decoded by this parser into a normal string.
 *  - shellToInvoke:   e.g. "/bin/sh", "/bin/bash", "/usr/bin/env python3".
 *
 * Children:
 *  - <EnvironmentBuildable> — optional; supplies build-setting context for
 *    macro substitution in scriptText.
 */
export class ActionContent extends SchemeNode {
  static override readonly allowedChildren = ["EnvironmentBuildable"] as const;

  get title(): string | undefined {
    return this.getString("title");
  }
  set title(v: string | undefined) {
    this.setString("title", v);
  }

  get scriptText(): string | undefined {
    return this.getString("scriptText");
  }
  set scriptText(v: string | undefined) {
    this.setString("scriptText", v);
  }

  get shellToInvoke(): string | undefined {
    return this.getString("shellToInvoke");
  }
  set shellToInvoke(v: string | undefined) {
    this.setString("shellToInvoke", v);
  }

  environmentBuildable(): EnvironmentBuildable | undefined {
    return this.getChild<EnvironmentBuildable>("EnvironmentBuildable");
  }
  setEnvironmentBuildable(node: EnvironmentBuildable | undefined): void {
    this.setChild("EnvironmentBuildable", node);
  }
}

/**
 * <ExecutionAction> — one pre/post action. The kind of action is identified
 * by `ActionType`. The only commonly-seen type is the shell-script action:
 *
 *   "Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction"
 *
 * which expects an <ActionContent> child holding the script.
 */
export class ExecutionAction extends SchemeNode {
  static override readonly allowedChildren = ["ActionContent"] as const;

  get actionType(): string | undefined {
    return this.getString("ActionType");
  }
  set actionType(v: string | undefined) {
    this.setString("ActionType", v);
  }

  actionContent(): ActionContent | undefined {
    return this.getChild<ActionContent>("ActionContent");
  }
  setActionContent(node: ActionContent | undefined): void {
    this.setChild("ActionContent", node);
  }
}

/**
 * <BuildActionEntry> — one target's participation in the scheme's build.
 * Lives inside <BuildActionEntries> on BuildAction.
 *
 * The five buildFor* booleans match the columns in Xcode's "Targets" table
 * in the Build action editor. Setting all to NO effectively excludes the
 * target from the scheme.
 */
export class BuildActionEntry extends SchemeNode {
  static override readonly allowedChildren = ["BuildableReference"] as const;

  get buildForTesting(): boolean | undefined {
    return this.getBool("buildForTesting");
  }
  set buildForTesting(v: boolean | undefined) {
    this.setBool("buildForTesting", v);
  }

  get buildForRunning(): boolean | undefined {
    return this.getBool("buildForRunning");
  }
  set buildForRunning(v: boolean | undefined) {
    this.setBool("buildForRunning", v);
  }

  get buildForProfiling(): boolean | undefined {
    return this.getBool("buildForProfiling");
  }
  set buildForProfiling(v: boolean | undefined) {
    this.setBool("buildForProfiling", v);
  }

  get buildForArchiving(): boolean | undefined {
    return this.getBool("buildForArchiving");
  }
  set buildForArchiving(v: boolean | undefined) {
    this.setBool("buildForArchiving", v);
  }

  get buildForAnalyzing(): boolean | undefined {
    return this.getBool("buildForAnalyzing");
  }
  set buildForAnalyzing(v: boolean | undefined) {
    this.setBool("buildForAnalyzing", v);
  }

  buildableReference(): BuildableReference | undefined {
    return this.getChild<BuildableReference>("BuildableReference");
  }
  setBuildableReference(ref: BuildableReference | undefined): void {
    this.setChild("BuildableReference", ref);
  }
}

/**
 * <BuildAction> — the "Build" tab of the scheme editor. Controls global
 * build behavior + the per-target participation matrix.
 *
 *  - parallelizeBuildables:     YES = build independent targets in parallel.
 *                               YES is the modern default; legacy schemes
 *                               sometimes still have NO.
 *  - buildImplicitDependencies: YES = follow framework / linker references
 *                               to also build dependencies that aren't
 *                               explicitly listed in the targets matrix.
 *  - runPostActionsOnFailure:   YES = run <PostActions> even if the build
 *                               fails (useful for cleanup scripts).
 *
 * Children: optional <PreActions> / <PostActions>, plus <BuildActionEntries>
 * containing one <BuildActionEntry> per target in the scheme.
 */
export class BuildAction extends SchemeNode {
  static override readonly allowedChildren = ["PreActions", "PostActions", "BuildActionEntries"] as const;

  get parallelizeBuildables(): boolean | undefined {
    return this.getBool("parallelizeBuildables");
  }
  set parallelizeBuildables(v: boolean | undefined) {
    this.setBool("parallelizeBuildables", v);
  }

  get buildImplicitDependencies(): boolean | undefined {
    return this.getBool("buildImplicitDependencies");
  }
  set buildImplicitDependencies(v: boolean | undefined) {
    this.setBool("buildImplicitDependencies", v);
  }

  get runPostActionsOnFailure(): boolean | undefined {
    return this.getBool("runPostActionsOnFailure");
  }
  set runPostActionsOnFailure(v: boolean | undefined) {
    this.setBool("runPostActionsOnFailure", v);
  }

  preActions(): ExecutionAction[] {
    return containerList(this, "PreActions", "ExecutionAction");
  }
  addPreAction(action: ExecutionAction): void {
    containerAppend(this, "PreActions", "ExecutionAction", action);
  }
  clearPreActions(): void {
    this.removeAllChildren("PreActions");
  }

  postActions(): ExecutionAction[] {
    return containerList(this, "PostActions", "ExecutionAction");
  }
  addPostAction(action: ExecutionAction): void {
    containerAppend(this, "PostActions", "ExecutionAction", action);
  }
  clearPostActions(): void {
    this.removeAllChildren("PostActions");
  }

  entries(): BuildActionEntry[] {
    return containerList(this, "BuildActionEntries", "BuildActionEntry");
  }
  addEntry(entry: BuildActionEntry): void {
    containerAppend(this, "BuildActionEntries", "BuildActionEntry", entry);
  }
  clearEntries(): void {
    this.removeAllChildren("BuildActionEntries");
  }
}

/**
 * <TestableReference> — wraps a single test target with run-time options.
 * Lives inside <Testables> on TestAction. One TestableReference per test
 * bundle (.xctest).
 *
 *  - skipped:                    YES = the bundle is in the scheme but won't
 *                                run by default. Used by Xcode's UI to
 *                                gray-out tests without removing them.
 *  - parallelizable:             YES = tests in this bundle run in parallel
 *                                (Xcode 10+).
 *  - useTestSelectionWhitelist:  YES = run only tests in <SelectedTests>,
 *                                ignore all others. NO (default) = run
 *                                everything not in <SkippedTests>.
 *  - testExecutionOrdering:      "random" / undefined (sequential).
 *
 * Children:
 *  - <BuildableReference>          identifies the test target.
 *  - <SkippedTests>                tests to exclude.
 *  - <SelectedTests>               (alternative to SkippedTests) tests to
 *                                  exclusively run.
 *  - <LocationScenarioReference>   per-testable GPS scenario (rare).
 */
export class TestableReference extends SchemeNode {
  static override readonly allowedChildren = [
    "BuildableReference",
    "SkippedTests",
    "SelectedTests",
    "LocationScenarioReference",
  ] as const;

  get skipped(): boolean | undefined {
    return this.getBool("skipped");
  }
  set skipped(v: boolean | undefined) {
    this.setBool("skipped", v);
  }

  get parallelizable(): boolean | undefined {
    return this.getBool("parallelizable");
  }
  set parallelizable(v: boolean | undefined) {
    this.setBool("parallelizable", v);
  }

  get useTestSelectionWhitelist(): boolean | undefined {
    return this.getBool("useTestSelectionWhitelist");
  }
  set useTestSelectionWhitelist(v: boolean | undefined) {
    this.setBool("useTestSelectionWhitelist", v);
  }

  get testExecutionOrdering(): string | undefined {
    return this.getString("testExecutionOrdering");
  }
  set testExecutionOrdering(v: string | undefined) {
    this.setString("testExecutionOrdering", v);
  }

  buildableReference(): BuildableReference | undefined {
    return this.getChild<BuildableReference>("BuildableReference");
  }
  setBuildableReference(ref: BuildableReference | undefined): void {
    this.setChild("BuildableReference", ref);
  }

  skippedTests(): TestItem[] {
    return containerList(this, "SkippedTests", "Test");
  }
  addSkippedTest(item: TestItem): void {
    containerAppend(this, "SkippedTests", "Test", item);
  }
  clearSkippedTests(): void {
    this.removeAllChildren("SkippedTests");
  }

  selectedTests(): TestItem[] {
    return containerList(this, "SelectedTests", "Test");
  }
  addSelectedTest(item: TestItem): void {
    containerAppend(this, "SelectedTests", "Test", item);
  }
  clearSelectedTests(): void {
    this.removeAllChildren("SelectedTests");
  }

  locationScenarioReference(): LocationScenarioReference | undefined {
    return this.getChild<LocationScenarioReference>("LocationScenarioReference");
  }
  setLocationScenarioReference(ref: LocationScenarioReference | undefined): void {
    this.setChild("LocationScenarioReference", ref);
  }
}

/**
 * <TestAction> — the "Test" tab of the scheme editor.
 *
 *  - buildConfiguration:                     Debug / Release / custom.
 *  - selectedDebuggerIdentifier:             usually LLDB.
 *  - selectedLauncherIdentifier:             usually LLDB.
 *  - shouldUseLaunchSchemeArgsEnv:           YES = tests inherit args/env from
 *                                            LaunchAction; NO = use the args/
 *                                            env defined here on TestAction.
 *  - codeCoverageEnabled:                    enable code coverage collection.
 *  - onlyGenerateCoverageForSpecifiedTargets: limit coverage to
 *                                            <CodeCoverageTargets>.
 *  - enableAddressSanitizer / UB / Thread:   runtime sanitizers (ASan/UBSan/TSan).
 *  - enableASanStackUseAfterReturn:          ASan sub-option, requires Address.
 *  - enableMallocStackLogging /
 *    enableMallocScribble /
 *    enableMallocGuardEdges /
 *    enableGuardMalloc:                      legacy heap-debugging knobs.
 *  - disableMainThreadChecker /
 *    disablePerformanceAntipatternChecker:   opt OUT of Xcode's default checks.
 *  - language / region:                      App Language and App Region for
 *                                            the test run (BCP-47 / ISO 3166-1).
 *  - systemAttachmentLifetime /
 *    userAttachmentLifetime:                 attachment retention policy:
 *                                            "keepNever" / "deleteOnSuccess" /
 *                                            "keepAlways".
 *
 * Children: PreActions, PostActions, MacroExpansion, CommandLineArguments,
 * EnvironmentVariables, AdditionalOptions, CodeCoverageTargets, TestPlans,
 * Testables.
 */
export class TestAction extends SchemeNode {
  static override readonly allowedChildren = [
    "PreActions",
    "PostActions",
    "MacroExpansion",
    "CommandLineArguments",
    "EnvironmentVariables",
    "AdditionalOptions",
    "CodeCoverageTargets",
    "TestPlans",
    "Testables",
  ] as const;

  get buildConfiguration(): string | undefined {
    return this.getString("buildConfiguration");
  }
  set buildConfiguration(v: string | undefined) {
    this.setString("buildConfiguration", v);
  }

  get selectedDebuggerIdentifier(): string | undefined {
    return this.getString("selectedDebuggerIdentifier");
  }
  set selectedDebuggerIdentifier(v: string | undefined) {
    this.setString("selectedDebuggerIdentifier", v);
  }

  get selectedLauncherIdentifier(): string | undefined {
    return this.getString("selectedLauncherIdentifier");
  }
  set selectedLauncherIdentifier(v: string | undefined) {
    this.setString("selectedLauncherIdentifier", v);
  }

  get shouldUseLaunchSchemeArgsEnv(): boolean | undefined {
    return this.getBool("shouldUseLaunchSchemeArgsEnv");
  }
  set shouldUseLaunchSchemeArgsEnv(v: boolean | undefined) {
    this.setBool("shouldUseLaunchSchemeArgsEnv", v);
  }

  get codeCoverageEnabled(): boolean | undefined {
    return this.getBool("codeCoverageEnabled");
  }
  set codeCoverageEnabled(v: boolean | undefined) {
    this.setBool("codeCoverageEnabled", v);
  }

  get onlyGenerateCoverageForSpecifiedTargets(): boolean | undefined {
    return this.getBool("onlyGenerateCoverageForSpecifiedTargets");
  }
  set onlyGenerateCoverageForSpecifiedTargets(v: boolean | undefined) {
    this.setBool("onlyGenerateCoverageForSpecifiedTargets", v);
  }

  get enableAddressSanitizer(): boolean | undefined {
    return this.getBool("enableAddressSanitizer");
  }
  set enableAddressSanitizer(v: boolean | undefined) {
    this.setBool("enableAddressSanitizer", v);
  }

  get enableASanStackUseAfterReturn(): boolean | undefined {
    return this.getBool("enableASanStackUseAfterReturn");
  }
  set enableASanStackUseAfterReturn(v: boolean | undefined) {
    this.setBool("enableASanStackUseAfterReturn", v);
  }

  get enableUBSanitizer(): boolean | undefined {
    return this.getBool("enableUBSanitizer");
  }
  set enableUBSanitizer(v: boolean | undefined) {
    this.setBool("enableUBSanitizer", v);
  }

  get enableThreadSanitizer(): boolean | undefined {
    return this.getBool("enableThreadSanitizer");
  }
  set enableThreadSanitizer(v: boolean | undefined) {
    this.setBool("enableThreadSanitizer", v);
  }

  get enableMallocStackLogging(): boolean | undefined {
    return this.getBool("enableMallocStackLogging");
  }
  set enableMallocStackLogging(v: boolean | undefined) {
    this.setBool("enableMallocStackLogging", v);
  }

  get enableMallocScribble(): boolean | undefined {
    return this.getBool("enableMallocScribble");
  }
  set enableMallocScribble(v: boolean | undefined) {
    this.setBool("enableMallocScribble", v);
  }

  get enableMallocGuardEdges(): boolean | undefined {
    return this.getBool("enableMallocGuardEdges");
  }
  set enableMallocGuardEdges(v: boolean | undefined) {
    this.setBool("enableMallocGuardEdges", v);
  }

  get enableGuardMalloc(): boolean | undefined {
    return this.getBool("enableGuardMalloc");
  }
  set enableGuardMalloc(v: boolean | undefined) {
    this.setBool("enableGuardMalloc", v);
  }

  get disableMainThreadChecker(): boolean | undefined {
    return this.getBool("disableMainThreadChecker");
  }
  set disableMainThreadChecker(v: boolean | undefined) {
    this.setBool("disableMainThreadChecker", v);
  }

  get disablePerformanceAntipatternChecker(): boolean | undefined {
    return this.getBool("disablePerformanceAntipatternChecker");
  }
  set disablePerformanceAntipatternChecker(v: boolean | undefined) {
    this.setBool("disablePerformanceAntipatternChecker", v);
  }

  get language(): string | undefined {
    return this.getString("language");
  }
  set language(v: string | undefined) {
    this.setString("language", v);
  }

  get region(): string | undefined {
    return this.getString("region");
  }
  set region(v: string | undefined) {
    this.setString("region", v);
  }

  get systemAttachmentLifetime(): string | undefined {
    return this.getString("systemAttachmentLifetime");
  }
  set systemAttachmentLifetime(v: string | undefined) {
    this.setString("systemAttachmentLifetime", v);
  }

  get userAttachmentLifetime(): string | undefined {
    return this.getString("userAttachmentLifetime");
  }
  set userAttachmentLifetime(v: string | undefined) {
    this.setString("userAttachmentLifetime", v);
  }

  preActions(): ExecutionAction[] {
    return containerList(this, "PreActions", "ExecutionAction");
  }
  addPreAction(a: ExecutionAction): void {
    containerAppend(this, "PreActions", "ExecutionAction", a);
  }
  clearPreActions(): void {
    this.removeAllChildren("PreActions");
  }

  postActions(): ExecutionAction[] {
    return containerList(this, "PostActions", "ExecutionAction");
  }
  addPostAction(a: ExecutionAction): void {
    containerAppend(this, "PostActions", "ExecutionAction", a);
  }
  clearPostActions(): void {
    this.removeAllChildren("PostActions");
  }

  macroExpansion(): MacroExpansion | undefined {
    return this.getChild<MacroExpansion>("MacroExpansion");
  }
  setMacroExpansion(m: MacroExpansion | undefined): void {
    this.setChild("MacroExpansion", m);
  }

  commandLineArguments(): CommandLineArgument[] {
    return containerList(this, "CommandLineArguments", "CommandLineArgument");
  }
  addCommandLineArgument(arg: CommandLineArgument | { argument: string; isEnabled?: boolean }): void {
    const item = arg instanceof CommandLineArgument ? arg : commandLineArgumentFrom(arg);
    containerAppend(this, "CommandLineArguments", "CommandLineArgument", item);
  }
  clearCommandLineArguments(): void {
    this.removeAllChildren("CommandLineArguments");
  }

  environmentVariables(): EnvironmentVariable[] {
    return containerList(this, "EnvironmentVariables", "EnvironmentVariable");
  }
  addEnvironmentVariable(env: EnvironmentVariable | { key: string; value: string; isEnabled?: boolean }): void {
    const item = env instanceof EnvironmentVariable ? env : environmentVariableFrom(env);
    containerAppend(this, "EnvironmentVariables", "EnvironmentVariable", item);
  }
  clearEnvironmentVariables(): void {
    this.removeAllChildren("EnvironmentVariables");
  }

  additionalOptions(): AdditionalOption[] {
    return containerList(this, "AdditionalOptions", "AdditionalOption");
  }
  addAdditionalOption(opt: AdditionalOption | { key: string; value: string; isEnabled?: boolean }): void {
    const item = opt instanceof AdditionalOption ? opt : additionalOptionFrom(opt);
    containerAppend(this, "AdditionalOptions", "AdditionalOption", item);
  }
  clearAdditionalOptions(): void {
    this.removeAllChildren("AdditionalOptions");
  }

  codeCoverageTargets(): BuildableReference[] {
    return containerList(this, "CodeCoverageTargets", "BuildableReference");
  }
  addCodeCoverageTarget(ref: BuildableReference): void {
    containerAppend(this, "CodeCoverageTargets", "BuildableReference", ref);
  }
  clearCodeCoverageTargets(): void {
    this.removeAllChildren("CodeCoverageTargets");
  }

  testPlans(): TestPlanReference[] {
    return containerList(this, "TestPlans", "TestPlanReference");
  }
  addTestPlan(plan: TestPlanReference): void {
    containerAppend(this, "TestPlans", "TestPlanReference", plan);
  }
  clearTestPlans(): void {
    this.removeAllChildren("TestPlans");
  }

  testables(): TestableReference[] {
    return containerList(this, "Testables", "TestableReference");
  }
  addTestable(testable: TestableReference): void {
    containerAppend(this, "Testables", "TestableReference", testable);
  }
  clearTestables(): void {
    this.removeAllChildren("Testables");
  }
}

/**
 * <LaunchAction> — the Run action's full configuration.
 *
 * Largest action class. This is the action discussion #197 cares about —
 * App Language for RTL testing, launch args, env vars.
 *
 * Build / debug:
 *  - buildConfiguration:               Debug / Release / custom.
 *  - selectedDebuggerIdentifier:       Xcode.DebuggerFoundation.Debugger.LLDB
 *                                      or .GDB / .None.
 *  - selectedLauncherIdentifier:       Xcode.DebuggerFoundation.Launcher.LLDB
 *                                      or PosixSpawn / .None.
 *  - launchStyle:                      "0" = auto (launch immediately),
 *                                      "1" = wait for executable to launch
 *                                      manually (used for daemons / xpc).
 *  - useCustomWorkingDirectory:        YES = use customWorkingDirectory below.
 *  - customWorkingDirectory:           path the process starts in.
 *  - ignoresPersistentStateOnLaunch:   YES = wipe app state each launch.
 *  - debugDocumentVersioning:          YES = NSDocument version logging.
 *  - debugServiceExtension:            "internal" / "external" — debug network
 *                                      service extensions.
 *  - allowLocationSimulation:          YES = honor LocationScenarioReference.
 *
 * Locale (discussion #197):
 *  - language:                         App Language override, BCP-47
 *                                      (e.g. "ar", "he", "zh-Hans").
 *  - region:                           App Region override, ISO 3166-1
 *                                      alpha-2 (e.g. "SA", "IL", "CN").
 *
 * Sanitizers (modern):
 *  - enableAddressSanitizer:           ASan — heap/stack/global memory errors.
 *  - enableASanStackUseAfterReturn:    ASan sub-option, requires ASan.
 *  - enableUBSanitizer:                UBSan — undefined behavior.
 *  - enableThreadSanitizer:            TSan — data races.
 *  - stopOnEveryMainThreadCheckerIssue
 *  - stopOnEveryUBSanitizerIssue
 *  - stopOnEveryThreadSanitizerIssue:  break the debugger on every issue.
 *
 * Legacy malloc / threading diagnostics:
 *  - enableMallocStackLogging
 *  - enableMallocScribble
 *  - enableMallocGuardEdges
 *  - enableGuardMalloc
 *  - disableMainThreadChecker
 *  - disablePerformanceAntipatternChecker
 *
 * GPU validation (Metal apps):
 *  - enableGPUValidationMode:          "0" disabled, "1" enabled.
 *  - enableGPUFrameCaptureMode:        "0" disabled, "1" Metal,
 *                                      "2" OpenGL ES (legacy).
 *  - enableGPUShaderValidationMode:    "0" disabled, "1" enabled.
 *  - enableGPUAPIValidationMode:       "0" disabled, "1" enabled.
 *
 * Children (exactly ONE of the three runnable variants):
 *  - <BuildableProductRunnable>:       app target (the common case).
 *  - <RemoteRunnable>:                 remote-app debugging.
 *  - <MacroExpansion>:                 framework / library scheme (no launch,
 *                                      just provides build-setting context).
 *
 * Other children:
 *  - <PreActions> / <PostActions>:     scripts that run around launch.
 *  - <CommandLineArguments>:           argv passed to the launched process.
 *  - <EnvironmentVariables>:           env vars set on the launched process.
 *  - <AdditionalOptions>:              legacy diagnostic checkboxes.
 *  - <LocationScenarioReference>:      simulated GPS scenario.
 */
export class LaunchAction extends SchemeNode {
  static override readonly allowedChildren = [
    "PreActions",
    "PostActions",
    "BuildableProductRunnable",
    "RemoteRunnable",
    "MacroExpansion",
    "CommandLineArguments",
    "EnvironmentVariables",
    "AdditionalOptions",
    "LocationScenarioReference",
  ] as const;

  get buildConfiguration(): string | undefined {
    return this.getString("buildConfiguration");
  }
  set buildConfiguration(v: string | undefined) {
    this.setString("buildConfiguration", v);
  }

  get selectedDebuggerIdentifier(): string | undefined {
    return this.getString("selectedDebuggerIdentifier");
  }
  set selectedDebuggerIdentifier(v: string | undefined) {
    this.setString("selectedDebuggerIdentifier", v);
  }

  get selectedLauncherIdentifier(): string | undefined {
    return this.getString("selectedLauncherIdentifier");
  }
  set selectedLauncherIdentifier(v: string | undefined) {
    this.setString("selectedLauncherIdentifier", v);
  }

  get launchStyle(): string | undefined {
    return this.getString("launchStyle");
  }
  set launchStyle(v: string | undefined) {
    this.setString("launchStyle", v);
  }

  get useCustomWorkingDirectory(): boolean | undefined {
    return this.getBool("useCustomWorkingDirectory");
  }
  set useCustomWorkingDirectory(v: boolean | undefined) {
    this.setBool("useCustomWorkingDirectory", v);
  }

  get customWorkingDirectory(): string | undefined {
    return this.getString("customWorkingDirectory");
  }
  set customWorkingDirectory(v: string | undefined) {
    this.setString("customWorkingDirectory", v);
  }

  get ignoresPersistentStateOnLaunch(): boolean | undefined {
    return this.getBool("ignoresPersistentStateOnLaunch");
  }
  set ignoresPersistentStateOnLaunch(v: boolean | undefined) {
    this.setBool("ignoresPersistentStateOnLaunch", v);
  }

  get debugDocumentVersioning(): boolean | undefined {
    return this.getBool("debugDocumentVersioning");
  }
  set debugDocumentVersioning(v: boolean | undefined) {
    this.setBool("debugDocumentVersioning", v);
  }

  get debugServiceExtension(): string | undefined {
    return this.getString("debugServiceExtension");
  }
  set debugServiceExtension(v: string | undefined) {
    this.setString("debugServiceExtension", v);
  }

  get allowLocationSimulation(): boolean | undefined {
    return this.getBool("allowLocationSimulation");
  }
  set allowLocationSimulation(v: boolean | undefined) {
    this.setBool("allowLocationSimulation", v);
  }

  get language(): string | undefined {
    return this.getString("language");
  }
  set language(v: string | undefined) {
    this.setString("language", v);
  }

  get region(): string | undefined {
    return this.getString("region");
  }
  set region(v: string | undefined) {
    this.setString("region", v);
  }

  get enableAddressSanitizer(): boolean | undefined {
    return this.getBool("enableAddressSanitizer");
  }
  set enableAddressSanitizer(v: boolean | undefined) {
    this.setBool("enableAddressSanitizer", v);
  }

  get enableASanStackUseAfterReturn(): boolean | undefined {
    return this.getBool("enableASanStackUseAfterReturn");
  }
  set enableASanStackUseAfterReturn(v: boolean | undefined) {
    this.setBool("enableASanStackUseAfterReturn", v);
  }

  get enableUBSanitizer(): boolean | undefined {
    return this.getBool("enableUBSanitizer");
  }
  set enableUBSanitizer(v: boolean | undefined) {
    this.setBool("enableUBSanitizer", v);
  }

  get enableThreadSanitizer(): boolean | undefined {
    return this.getBool("enableThreadSanitizer");
  }
  set enableThreadSanitizer(v: boolean | undefined) {
    this.setBool("enableThreadSanitizer", v);
  }

  get enableMallocStackLogging(): boolean | undefined {
    return this.getBool("enableMallocStackLogging");
  }
  set enableMallocStackLogging(v: boolean | undefined) {
    this.setBool("enableMallocStackLogging", v);
  }

  get enableMallocScribble(): boolean | undefined {
    return this.getBool("enableMallocScribble");
  }
  set enableMallocScribble(v: boolean | undefined) {
    this.setBool("enableMallocScribble", v);
  }

  get enableMallocGuardEdges(): boolean | undefined {
    return this.getBool("enableMallocGuardEdges");
  }
  set enableMallocGuardEdges(v: boolean | undefined) {
    this.setBool("enableMallocGuardEdges", v);
  }

  get enableGuardMalloc(): boolean | undefined {
    return this.getBool("enableGuardMalloc");
  }
  set enableGuardMalloc(v: boolean | undefined) {
    this.setBool("enableGuardMalloc", v);
  }

  get disableMainThreadChecker(): boolean | undefined {
    return this.getBool("disableMainThreadChecker");
  }
  set disableMainThreadChecker(v: boolean | undefined) {
    this.setBool("disableMainThreadChecker", v);
  }

  get disablePerformanceAntipatternChecker(): boolean | undefined {
    return this.getBool("disablePerformanceAntipatternChecker");
  }
  set disablePerformanceAntipatternChecker(v: boolean | undefined) {
    this.setBool("disablePerformanceAntipatternChecker", v);
  }

  get stopOnEveryMainThreadCheckerIssue(): boolean | undefined {
    return this.getBool("stopOnEveryMainThreadCheckerIssue");
  }
  set stopOnEveryMainThreadCheckerIssue(v: boolean | undefined) {
    this.setBool("stopOnEveryMainThreadCheckerIssue", v);
  }

  get stopOnEveryUBSanitizerIssue(): boolean | undefined {
    return this.getBool("stopOnEveryUBSanitizerIssue");
  }
  set stopOnEveryUBSanitizerIssue(v: boolean | undefined) {
    this.setBool("stopOnEveryUBSanitizerIssue", v);
  }

  get stopOnEveryThreadSanitizerIssue(): boolean | undefined {
    return this.getBool("stopOnEveryThreadSanitizerIssue");
  }
  set stopOnEveryThreadSanitizerIssue(v: boolean | undefined) {
    this.setBool("stopOnEveryThreadSanitizerIssue", v);
  }

  get enableGPUValidationMode(): string | undefined {
    return this.getString("enableGPUValidationMode");
  }
  set enableGPUValidationMode(v: string | undefined) {
    this.setString("enableGPUValidationMode", v);
  }

  get enableGPUFrameCaptureMode(): string | undefined {
    return this.getString("enableGPUFrameCaptureMode");
  }
  set enableGPUFrameCaptureMode(v: string | undefined) {
    this.setString("enableGPUFrameCaptureMode", v);
  }

  get enableGPUShaderValidationMode(): string | undefined {
    return this.getString("enableGPUShaderValidationMode");
  }
  set enableGPUShaderValidationMode(v: string | undefined) {
    this.setString("enableGPUShaderValidationMode", v);
  }

  get enableGPUAPIValidationMode(): string | undefined {
    return this.getString("enableGPUAPIValidationMode");
  }
  set enableGPUAPIValidationMode(v: string | undefined) {
    this.setString("enableGPUAPIValidationMode", v);
  }

  preActions(): ExecutionAction[] {
    return containerList(this, "PreActions", "ExecutionAction");
  }
  addPreAction(a: ExecutionAction): void {
    containerAppend(this, "PreActions", "ExecutionAction", a);
  }
  clearPreActions(): void {
    this.removeAllChildren("PreActions");
  }

  postActions(): ExecutionAction[] {
    return containerList(this, "PostActions", "ExecutionAction");
  }
  addPostAction(a: ExecutionAction): void {
    containerAppend(this, "PostActions", "ExecutionAction", a);
  }
  clearPostActions(): void {
    this.removeAllChildren("PostActions");
  }

  buildableProductRunnable(): BuildableProductRunnable | undefined {
    return this.getChild<BuildableProductRunnable>("BuildableProductRunnable");
  }
  setBuildableProductRunnable(r: BuildableProductRunnable | undefined): void {
    this.setChild("BuildableProductRunnable", r);
  }

  remoteRunnable(): RemoteRunnable | undefined {
    return this.getChild<RemoteRunnable>("RemoteRunnable");
  }
  setRemoteRunnable(r: RemoteRunnable | undefined): void {
    this.setChild("RemoteRunnable", r);
  }

  macroExpansion(): MacroExpansion | undefined {
    return this.getChild<MacroExpansion>("MacroExpansion");
  }
  setMacroExpansion(m: MacroExpansion | undefined): void {
    this.setChild("MacroExpansion", m);
  }

  commandLineArguments(): CommandLineArgument[] {
    return containerList(this, "CommandLineArguments", "CommandLineArgument");
  }
  addCommandLineArgument(arg: CommandLineArgument | { argument: string; isEnabled?: boolean }): void {
    const item = arg instanceof CommandLineArgument ? arg : commandLineArgumentFrom(arg);
    containerAppend(this, "CommandLineArguments", "CommandLineArgument", item);
  }
  clearCommandLineArguments(): void {
    this.removeAllChildren("CommandLineArguments");
  }

  environmentVariables(): EnvironmentVariable[] {
    return containerList(this, "EnvironmentVariables", "EnvironmentVariable");
  }
  addEnvironmentVariable(env: EnvironmentVariable | { key: string; value: string; isEnabled?: boolean }): void {
    const item = env instanceof EnvironmentVariable ? env : environmentVariableFrom(env);
    containerAppend(this, "EnvironmentVariables", "EnvironmentVariable", item);
  }
  clearEnvironmentVariables(): void {
    this.removeAllChildren("EnvironmentVariables");
  }

  additionalOptions(): AdditionalOption[] {
    return containerList(this, "AdditionalOptions", "AdditionalOption");
  }
  addAdditionalOption(opt: AdditionalOption | { key: string; value: string; isEnabled?: boolean }): void {
    const item = opt instanceof AdditionalOption ? opt : additionalOptionFrom(opt);
    containerAppend(this, "AdditionalOptions", "AdditionalOption", item);
  }
  clearAdditionalOptions(): void {
    this.removeAllChildren("AdditionalOptions");
  }

  locationScenarioReference(): LocationScenarioReference | undefined {
    return this.getChild<LocationScenarioReference>("LocationScenarioReference");
  }
  setLocationScenarioReference(r: LocationScenarioReference | undefined): void {
    this.setChild("LocationScenarioReference", r);
  }

  // --- domain convenience -------------------------------------------------

  /** Name of the target this LaunchAction will run, or null for framework / remote / macro schemes. */
  launchTarget(): string | null {
    return this.buildableProductRunnable()?.buildableReference()?.blueprintName ?? null;
  }

  /** App Language + App Region override (discussion #197 use case). */
  setAppLocale(language: string | undefined, region?: string | undefined): void {
    this.language = language;
    this.region = region;
  }
}

/**
 * <ProfileAction> — the "Profile" tab (launches in Instruments).
 *
 *  - shouldUseLaunchSchemeArgsEnv:  YES = inherit args/env from LaunchAction
 *                                   (the common case — keep Profile in sync
 *                                   with Run). NO = use args/env defined
 *                                   directly on this ProfileAction.
 *  - savedToolIdentifier:           empty string by default; an Instruments
 *                                   tool ID if the user pinned a specific
 *                                   template (e.g. Time Profiler).
 *
 * Other attrs mirror LaunchAction's launch/debug knobs. Children are the
 * same runnable variants as LaunchAction.
 */
export class ProfileAction extends SchemeNode {
  static override readonly allowedChildren = [
    "PreActions",
    "PostActions",
    "BuildableProductRunnable",
    "RemoteRunnable",
    "MacroExpansion",
    "CommandLineArguments",
    "EnvironmentVariables",
  ] as const;

  get buildConfiguration(): string | undefined {
    return this.getString("buildConfiguration");
  }
  set buildConfiguration(v: string | undefined) {
    this.setString("buildConfiguration", v);
  }

  get shouldUseLaunchSchemeArgsEnv(): boolean | undefined {
    return this.getBool("shouldUseLaunchSchemeArgsEnv");
  }
  set shouldUseLaunchSchemeArgsEnv(v: boolean | undefined) {
    this.setBool("shouldUseLaunchSchemeArgsEnv", v);
  }

  get savedToolIdentifier(): string | undefined {
    return this.getString("savedToolIdentifier");
  }
  set savedToolIdentifier(v: string | undefined) {
    this.setString("savedToolIdentifier", v);
  }

  get useCustomWorkingDirectory(): boolean | undefined {
    return this.getBool("useCustomWorkingDirectory");
  }
  set useCustomWorkingDirectory(v: boolean | undefined) {
    this.setBool("useCustomWorkingDirectory", v);
  }

  get customWorkingDirectory(): string | undefined {
    return this.getString("customWorkingDirectory");
  }
  set customWorkingDirectory(v: string | undefined) {
    this.setString("customWorkingDirectory", v);
  }

  get debugDocumentVersioning(): boolean | undefined {
    return this.getBool("debugDocumentVersioning");
  }
  set debugDocumentVersioning(v: boolean | undefined) {
    this.setBool("debugDocumentVersioning", v);
  }

  preActions(): ExecutionAction[] {
    return containerList(this, "PreActions", "ExecutionAction");
  }
  addPreAction(a: ExecutionAction): void {
    containerAppend(this, "PreActions", "ExecutionAction", a);
  }
  clearPreActions(): void {
    this.removeAllChildren("PreActions");
  }

  postActions(): ExecutionAction[] {
    return containerList(this, "PostActions", "ExecutionAction");
  }
  addPostAction(a: ExecutionAction): void {
    containerAppend(this, "PostActions", "ExecutionAction", a);
  }
  clearPostActions(): void {
    this.removeAllChildren("PostActions");
  }

  buildableProductRunnable(): BuildableProductRunnable | undefined {
    return this.getChild<BuildableProductRunnable>("BuildableProductRunnable");
  }
  setBuildableProductRunnable(r: BuildableProductRunnable | undefined): void {
    this.setChild("BuildableProductRunnable", r);
  }

  remoteRunnable(): RemoteRunnable | undefined {
    return this.getChild<RemoteRunnable>("RemoteRunnable");
  }
  setRemoteRunnable(r: RemoteRunnable | undefined): void {
    this.setChild("RemoteRunnable", r);
  }

  macroExpansion(): MacroExpansion | undefined {
    return this.getChild<MacroExpansion>("MacroExpansion");
  }
  setMacroExpansion(m: MacroExpansion | undefined): void {
    this.setChild("MacroExpansion", m);
  }

  commandLineArguments(): CommandLineArgument[] {
    return containerList(this, "CommandLineArguments", "CommandLineArgument");
  }
  addCommandLineArgument(arg: CommandLineArgument | { argument: string; isEnabled?: boolean }): void {
    const item = arg instanceof CommandLineArgument ? arg : commandLineArgumentFrom(arg);
    containerAppend(this, "CommandLineArguments", "CommandLineArgument", item);
  }
  clearCommandLineArguments(): void {
    this.removeAllChildren("CommandLineArguments");
  }

  environmentVariables(): EnvironmentVariable[] {
    return containerList(this, "EnvironmentVariables", "EnvironmentVariable");
  }
  addEnvironmentVariable(env: EnvironmentVariable | { key: string; value: string; isEnabled?: boolean }): void {
    const item = env instanceof EnvironmentVariable ? env : environmentVariableFrom(env);
    containerAppend(this, "EnvironmentVariables", "EnvironmentVariable", item);
  }
  clearEnvironmentVariables(): void {
    this.removeAllChildren("EnvironmentVariables");
  }
}

/**
 * <AnalyzeAction> — the "Analyze" tab (Clang static analyzer). Tiny: just a
 * build configuration plus optional pre/post actions.
 */
export class AnalyzeAction extends SchemeNode {
  static override readonly allowedChildren = ["PreActions", "PostActions"] as const;

  get buildConfiguration(): string | undefined {
    return this.getString("buildConfiguration");
  }
  set buildConfiguration(v: string | undefined) {
    this.setString("buildConfiguration", v);
  }

  preActions(): ExecutionAction[] {
    return containerList(this, "PreActions", "ExecutionAction");
  }
  addPreAction(a: ExecutionAction): void {
    containerAppend(this, "PreActions", "ExecutionAction", a);
  }
  clearPreActions(): void {
    this.removeAllChildren("PreActions");
  }

  postActions(): ExecutionAction[] {
    return containerList(this, "PostActions", "ExecutionAction");
  }
  addPostAction(a: ExecutionAction): void {
    containerAppend(this, "PostActions", "ExecutionAction", a);
  }
  clearPostActions(): void {
    this.removeAllChildren("PostActions");
  }
}

/**
 * <ArchiveAction> — the "Archive" tab. Produces the .xcarchive for App Store
 * distribution.
 *
 *  - revealArchiveInOrganizer:  YES = open the Organizer window after archive.
 *  - customArchiveName:         override the auto-generated archive name
 *                               (which is normally "$(TARGET_NAME) $(NOW)").
 */
export class ArchiveAction extends SchemeNode {
  static override readonly allowedChildren = ["PreActions", "PostActions"] as const;

  get buildConfiguration(): string | undefined {
    return this.getString("buildConfiguration");
  }
  set buildConfiguration(v: string | undefined) {
    this.setString("buildConfiguration", v);
  }

  get revealArchiveInOrganizer(): boolean | undefined {
    return this.getBool("revealArchiveInOrganizer");
  }
  set revealArchiveInOrganizer(v: boolean | undefined) {
    this.setBool("revealArchiveInOrganizer", v);
  }

  get customArchiveName(): string | undefined {
    return this.getString("customArchiveName");
  }
  set customArchiveName(v: string | undefined) {
    this.setString("customArchiveName", v);
  }

  preActions(): ExecutionAction[] {
    return containerList(this, "PreActions", "ExecutionAction");
  }
  addPreAction(a: ExecutionAction): void {
    containerAppend(this, "PreActions", "ExecutionAction", a);
  }
  clearPreActions(): void {
    this.removeAllChildren("PreActions");
  }

  postActions(): ExecutionAction[] {
    return containerList(this, "PostActions", "ExecutionAction");
  }
  addPostAction(a: ExecutionAction): void {
    containerAppend(this, "PostActions", "ExecutionAction", a);
  }
  clearPostActions(): void {
    this.removeAllChildren("PostActions");
  }
}

/**
 * <Scheme> — the root of an .xcscheme file.
 *
 *  - LastUpgradeVersion:         Xcode version that last upgraded this file
 *                                (e.g. "1640" for Xcode 16.4). Bumped each
 *                                time Xcode opens-and-rewrites the scheme;
 *                                useful as a "what generated this" marker.
 *  - version:                    Apple's internal XML schema version
 *                                ("1.3" / "1.7" in current schemes).
 *  - wasCreatedForAppExtension:  YES if created by Xcode's New Scheme wizard
 *                                from an app-extension target.
 *
 * Children — one of each at most, in this order (Xcode-canonical):
 *  BuildAction, TestAction, LaunchAction, ProfileAction, AnalyzeAction,
 *  ArchiveAction.
 *
 * Plus this class carries `xmlDeclaration` (the <?xml ...?> line, not really
 * an XML element) and `schemaVersion` (the parser's model version, useful
 * for migrations in the future).
 */
export class SchemeDocument extends SchemeNode {
  static override readonly allowedChildren = [
    "BuildAction",
    "TestAction",
    "LaunchAction",
    "ProfileAction",
    "AnalyzeAction",
    "ArchiveAction",
  ] as const;

  /** Schema version of this parsed-document shape (not the XML `version` attribute). */
  readonly schemaVersion: string = SCHEME_DOCUMENT_VERSION;

  /** XML declaration. Defaults to `<?xml version="1.0" encoding="UTF-8"?>` for documents built from scratch. */
  xmlDeclaration: XmlDecl = { version: "1.0", encoding: "UTF-8" };

  get lastUpgradeVersion(): string | undefined {
    return this.getString("LastUpgradeVersion");
  }
  set lastUpgradeVersion(v: string | undefined) {
    this.setString("LastUpgradeVersion", v);
  }

  get version(): string | undefined {
    return this.getString("version");
  }
  set version(v: string | undefined) {
    this.setString("version", v);
  }

  get wasCreatedForAppExtension(): boolean | undefined {
    return this.getBool("wasCreatedForAppExtension");
  }
  set wasCreatedForAppExtension(v: boolean | undefined) {
    this.setBool("wasCreatedForAppExtension", v);
  }

  buildAction(): BuildAction | undefined {
    return this.getChild<BuildAction>("BuildAction");
  }
  setBuildAction(a: BuildAction | undefined): void {
    this.setChild("BuildAction", a);
  }

  testAction(): TestAction | undefined {
    return this.getChild<TestAction>("TestAction");
  }
  setTestAction(a: TestAction | undefined): void {
    this.setChild("TestAction", a);
  }

  launchAction(): LaunchAction | undefined {
    return this.getChild<LaunchAction>("LaunchAction");
  }
  setLaunchAction(a: LaunchAction | undefined): void {
    this.setChild("LaunchAction", a);
  }

  profileAction(): ProfileAction | undefined {
    return this.getChild<ProfileAction>("ProfileAction");
  }
  setProfileAction(a: ProfileAction | undefined): void {
    this.setChild("ProfileAction", a);
  }

  analyzeAction(): AnalyzeAction | undefined {
    return this.getChild<AnalyzeAction>("AnalyzeAction");
  }
  setAnalyzeAction(a: AnalyzeAction | undefined): void {
    this.setChild("AnalyzeAction", a);
  }

  archiveAction(): ArchiveAction | undefined {
    return this.getChild<ArchiveAction>("ArchiveAction");
  }
  setArchiveAction(a: ArchiveAction | undefined): void {
    this.setChild("ArchiveAction", a);
  }

  /** Name of the target that the scheme's Run action will launch, or null. */
  targetToLaunch(): string | null {
    return this.launchAction()?.launchTarget() ?? null;
  }

  static parse(xml: string): SchemeDocument {
    return parseSchemeXml(xml);
  }

  serialize(): string {
    return serializeSchemeDoc(this);
  }
}

interface ContainerAccess {
  getChild<U extends SchemeNode>(name: string): U | undefined;
  getChildList<U extends SchemeNode>(name: string): U[];
  setChild(name: string, value: SchemeNode | undefined): void;
  appendChild(name: string, child: SchemeNode): void;
}

function containerList<T extends SchemeNode>(parent: SchemeNode, containerName: string, itemName: string): T[] {
  const accessor = parent as unknown as ContainerAccess;
  const container = accessor.getChild<ContainerNode>(containerName);
  if (!container) return [];
  return (container as unknown as ContainerAccess).getChildList<T>(itemName);
}

function containerAppend(parent: SchemeNode, containerName: string, itemName: string, item: SchemeNode): void {
  const accessor = parent as unknown as ContainerAccess;
  let container = accessor.getChild<ContainerNode>(containerName);
  if (!container) {
    container = new ContainerNode();
    accessor.setChild(containerName, container);
  }
  (container as unknown as ContainerAccess).appendChild(itemName, item);
}

function commandLineArgumentFrom(init: { argument: string; isEnabled?: boolean }): CommandLineArgument {
  const node = new CommandLineArgument();
  node.argument = init.argument;
  node.isEnabled = init.isEnabled ?? true;
  return node;
}

function environmentVariableFrom(init: { key: string; value: string; isEnabled?: boolean }): EnvironmentVariable {
  const node = new EnvironmentVariable();
  node.key = init.key;
  node.value = init.value;
  node.isEnabled = init.isEnabled ?? true;
  return node;
}

function additionalOptionFrom(init: { key: string; value: string; isEnabled?: boolean }): AdditionalOption {
  const node = new AdditionalOption();
  node.key = init.key;
  node.value = init.value;
  node.isEnabled = init.isEnabled ?? true;
  return node;
}

const CLASS_REGISTRY: Record<string, new () => SchemeNode> = {
  Scheme: SchemeDocument,
  BuildAction: BuildAction,
  BuildActionEntries: ContainerNode,
  BuildActionEntry: BuildActionEntry,
  TestAction: TestAction,
  Testables: ContainerNode,
  TestableReference: TestableReference,
  SkippedTests: ContainerNode,
  SelectedTests: ContainerNode,
  Test: TestItem,
  TestPlans: ContainerNode,
  TestPlanReference: TestPlanReference,
  CodeCoverageTargets: ContainerNode,
  LaunchAction: LaunchAction,
  ProfileAction: ProfileAction,
  AnalyzeAction: AnalyzeAction,
  ArchiveAction: ArchiveAction,
  BuildableReference: BuildableReference,
  BuildableProductRunnable: BuildableProductRunnable,
  RemoteRunnable: RemoteRunnable,
  MacroExpansion: MacroExpansion,
  EnvironmentBuildable: EnvironmentBuildable,
  CommandLineArguments: ContainerNode,
  CommandLineArgument: CommandLineArgument,
  EnvironmentVariables: ContainerNode,
  EnvironmentVariable: EnvironmentVariable,
  AdditionalOptions: ContainerNode,
  AdditionalOption: AdditionalOption,
  LocationScenarioReference: LocationScenarioReference,
  PreActions: ContainerNode,
  PostActions: ContainerNode,
  ExecutionAction: ExecutionAction,
  ActionContent: ActionContent,
};

/**
 * Element names that ContainerNode is allowed to wrap, by container name.
 * ContainerNode itself has no static schema (it's reused for many shapes), so
 * the parser consults this table for each container instance.
 */
const CONTAINER_ITEM_NAMES: Record<string, readonly string[]> = {
  BuildActionEntries: ["BuildActionEntry"],
  Testables: ["TestableReference"],
  SkippedTests: ["Test"],
  SelectedTests: ["Test"],
  TestPlans: ["TestPlanReference"],
  CodeCoverageTargets: ["BuildableReference"],
  CommandLineArguments: ["CommandLineArgument"],
  EnvironmentVariables: ["EnvironmentVariable"],
  AdditionalOptions: ["AdditionalOption"],
  PreActions: ["ExecutionAction"],
  PostActions: ["ExecutionAction"],
};

/** What element names are valid children for the given node instance. */
function allowedChildrenFor(node: SchemeNode, elementName: string): readonly string[] {
  if (node instanceof ContainerNode) {
    return CONTAINER_ITEM_NAMES[elementName] ?? [];
  }
  const ctor = node.constructor as typeof SchemeNode;
  return ctor.allowedChildren ?? [];
}

export function parseScheme(xml: string): SchemeDocument {
  return parseSchemeXml(xml);
}

function parseSchemeXml(xml: string): SchemeDocument {
  let parsed;
  try {
    parsed = parseXml(xml, {
      preserveCdata: true,
      preserveComments: true,
      preserveXmlDeclaration: true,
    });
  } catch (err) {
    if (err instanceof XmlError) {
      throw new XcSchemeParseError(`Invalid xcscheme XML: ${err.message}`, {
        line: err.line,
        column: err.column,
        sourceContext: contextSnippet(xml, err.line, err.column),
      });
    }
    throw new XcSchemeParseError(`Invalid xcscheme XML: ${(err as Error).message}`);
  }

  const root = parsed.root;
  if (!root || root.name !== "Scheme") {
    throw new XcSchemeParseError(`Invalid xcscheme: root element must be <Scheme>, got <${root?.name ?? "(none)"}>`);
  }

  const doc = new SchemeDocument();

  // XML declaration — capture if present.
  const decl = parsed.children.find((c): c is XmlDeclaration => c instanceof XmlDeclaration);
  if (decl) {
    doc.xmlDeclaration = {
      version: decl.version,
      encoding: decl.encoding ?? undefined,
      standalone: decl.standalone ?? undefined,
    };
  }

  hydrateNode(doc, root, "Scheme");
  return doc;
}

function hydrateNode(node: SchemeNode, el: XmlElement, elementName: string): void {
  // Attributes — in source order.
  for (const name of Object.keys(el.attributes)) {
    node._hydrateAttr(name, el.attributes[name]);
  }

  const allowed = new Set(allowedChildrenFor(node, elementName));

  for (const child of el.children) {
    if (child instanceof XmlElement) {
      if (allowed.has(child.name) && CLASS_REGISTRY[child.name]) {
        const ChildCtor = CLASS_REGISTRY[child.name];
        const childNode = new ChildCtor();
        hydrateNode(childNode, child, child.name);
        node._appendParsedChild(child.name, childNode);
      } else {
        node._appendParsedExtra(toGenericNode(child));
      }
    } else if (child instanceof XmlComment) {
      node._appendParsedComment(child.content);
    } else if (child instanceof XmlCdata) {
      node._appendParsedCdata(child.text);
    }
    // Whitespace-only text nodes are parser-internal noise in .xcscheme — drop.
  }
}

function toGenericNode(el: XmlElement): GenericNode {
  const attrs = new Map<string, string>();
  for (const k of Object.keys(el.attributes)) attrs.set(k, el.attributes[k]);
  const slots: ChildSlot[] = [];
  const children = new Map<string, GenericNode[]>();
  for (const child of el.children) {
    if (child instanceof XmlElement) {
      const g = toGenericNode(child);
      const list = children.get(child.name) ?? [];
      list.push(g);
      children.set(child.name, list);
      slots.push({ kind: "element", name: child.name, index: list.length - 1 });
    } else if (child instanceof XmlComment) {
      slots.push({ kind: "comment", text: child.content });
    } else if (child instanceof XmlCdata) {
      slots.push({ kind: "cdata", text: child.text });
    }
  }
  return { name: el.name, attrs, slots, children };
}

function contextSnippet(xml: string, line: number, column: number): string {
  const lines = xml.split("\n");
  const target = lines[line - 1] ?? "";
  const caret = " ".repeat(Math.max(0, column - 1)) + "^";
  return `  ${target}\n  ${caret}`;
}

const INDENT = "   "; // Xcode uses 3 spaces per nesting level.

export function serializeScheme(doc: SchemeDocument): string {
  return serializeSchemeDoc(doc);
}

function serializeSchemeDoc(doc: SchemeDocument): string {
  const lines: string[] = [];
  lines.push(formatXmlDeclaration(doc.xmlDeclaration));
  writeNode("Scheme", doc, 0, lines);
  lines.push("");
  return lines.join("\n");
}

function formatXmlDeclaration(decl: XmlDecl): string {
  const parts = [`version="${escapeAttrValue(decl.version)}"`];
  if (decl.encoding) parts.push(`encoding="${escapeAttrValue(decl.encoding)}"`);
  if (decl.standalone) parts.push(`standalone="${escapeAttrValue(decl.standalone)}"`);
  return `<?xml ${parts.join(" ")}?>`;
}

function writeNode(name: string, node: SchemeNode, depth: number, out: string[]): void {
  const pad = INDENT.repeat(depth);
  const childPad = INDENT.repeat(depth + 1);
  const attrs = node.attributeEntries();

  if (attrs.length === 0) {
    out.push(`${pad}<${name}>`);
  } else {
    out.push(`${pad}<${name}`);
    for (let i = 0; i < attrs.length; i++) {
      const [attrName, attrVal] = attrs[i];
      const suffix = i === attrs.length - 1 ? ">" : "";
      out.push(`${childPad}${attrName} = "${escapeAttrValue(attrVal)}"${suffix}`);
    }
  }

  for (const slot of node._slotsForSerialize()) {
    switch (slot.kind) {
      case "element": {
        const child = node._childAt(slot.name, slot.index);
        if (child) writeNode(slot.name, child, depth + 1, out);
        break;
      }
      case "comment":
        out.push(`${childPad}<!--${slot.text}-->`);
        break;
      case "cdata":
        out.push(`${childPad}<![CDATA[${slot.text}]]>`);
        break;
      case "extra":
        writeGenericNode(slot.node, depth + 1, out);
        break;
    }
  }

  out.push(`${pad}</${name}>`);
}

function writeGenericNode(node: GenericNode, depth: number, out: string[]): void {
  const pad = INDENT.repeat(depth);
  const childPad = INDENT.repeat(depth + 1);
  const attrs = Array.from(node.attrs.entries());

  if (attrs.length === 0) {
    out.push(`${pad}<${node.name}>`);
  } else {
    out.push(`${pad}<${node.name}`);
    for (let i = 0; i < attrs.length; i++) {
      const [attrName, attrVal] = attrs[i];
      const suffix = i === attrs.length - 1 ? ">" : "";
      out.push(`${childPad}${attrName} = "${escapeAttrValue(attrVal)}"${suffix}`);
    }
  }

  for (const slot of node.slots) {
    switch (slot.kind) {
      case "element": {
        const children = node.children.get(slot.name);
        const child = children?.[slot.index];
        if (child) writeGenericNode(child, depth + 1, out);
        break;
      }
      case "comment":
        out.push(`${childPad}<!--${slot.text}-->`);
        break;
      case "cdata":
        out.push(`${childPad}<![CDATA[${slot.text}]]>`);
        break;
      case "extra":
        writeGenericNode(slot.node, depth + 1, out);
        break;
    }
  }

  out.push(`${pad}</${node.name}>`);
}

function escapeAttrValue(value: string): string {
  let out = "";
  for (const ch of value) {
    const code = ch.codePointAt(0)!;
    if (ch === "&") out += "&amp;";
    else if (ch === "<") out += "&lt;";
    else if (ch === ">") out += "&gt;";
    else if (ch === '"') out += "&quot;";
    else if (ch === "'") out += "&apos;";
    else if (ch === "\n") out += "&#10;";
    else if (ch === "\r") out += "&#13;";
    else if (ch === "\t") out += "&#9;";
    else if (code < 0x20) out += `&#${code};`;
    else out += ch;
  }
  return out;
}

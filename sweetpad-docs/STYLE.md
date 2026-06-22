# SweetPad docs — writing style

Short guide for keeping the docs readable. Every rule here was learned the hard
way from a section that turned into a wall of red code-pills.

The goal: docs that *read like prose*, with code formatting reserved for things
the reader will actually type, copy, or grep for. The visual default is a soft
gray inline-code pill — quiet enough that ten pills in a paragraph don't add up
to a wall.

---

## 1. Code-format only what the reader will type

Inline `code` should mark things the reader interacts with mechanically: env
vars, build settings, CLI flags, settings keys, exact values they'll copy or
search for. If a token is a *concept* you're describing — a framework, a path
mentioned in passing, a Mach-O section name, a function in some third party
library — leave it as plain text.

```text
✅ Set `DYLD_INSERT_LIBRARIES` on the launched process.
✅ Pass `-Xlinker -interposable` to the linker.
✅ Add to `.vscode/settings.json`: …
✅ Search settings for "sweetpad hot reload".

❌ The runtime walks the `__la_symbol_ptr` and `__nl_symbol_ptr` sections.
❌ It re-invokes `swift-frontend`, the binary `swiftc` wraps.
❌ Reads logs from `/Users/<you>/Library/Developer/Xcode/DerivedData/...`.
```

Test: would the reader open a terminal and type this, or paste it into a
settings file? If no, drop the backticks.

### Special case: no inline code in headings

Markdown headings (`#`, `##`, `###`, etc.) must be plain text. No backticks at
all, even for identifiers that *would* be code-formatted in body text.
Code-pills inside a heading look awkward in the rendered page, in the sidebar
TOC, and in the auto-generated anchor slug.

```text
❌ ### Why `-Xlinker -interposable` is necessary
✅ ### Why -Xlinker -interposable is necessary

❌ ### Why `DYLD_INSERT_LIBRARIES` and not a code change
✅ ### Why DYLD_INSERT_LIBRARIES and not a code change
```

ALL_CAPS env vars and flags read fine as plain text in headings — the casing
and punctuation already make them stand out. If a heading feels like it needs
code formatting to be readable, rephrase the heading instead.

## 2. Never reference source code

No file paths from third-party repos, no function or method names, no GitHub
links to specific source files or line numbers. Source code rots. Describe
behavior conceptually instead.

```text
❌ See `recompile(source:platform:)` in [`NextCompiler.swift`](github link).
❌ The runtime auto-connects on `+load` from `ClientBoot.mm`.

✅ The runtime auto-connects to InjectionNext when the dylib loads.
```

Stable, top-level project links (a repo home page, a releases page) are fine —
they don't pin to a specific file or function. Anything deeper rots.

## 3. Break the wall

The longest sentence in a paragraph shouldn't fit in fewer breaths than the
reader has. The longest paragraph shouldn't be longer than ten lines on a
laptop screen. When you find yourself writing a 200-word paragraph with eight
code pills, that's the rule firing.

Three tactics, in order of preference:

- **Split into shorter paragraphs.** Each paragraph one idea. Blank lines do
  real work.
- **Convert to bullets** when the paragraph is enumerating things. One bullet
  per item, one short sentence per bullet.
- **Promote to sub-headings** when a numbered list grows past ~3 sentences per
  step. Each step gets its own `###`, which also gives the sidebar a navigable
  TOC.

## 4. Frame complex sections with an intro sentence

Before diving into "first this, then that, finally the third thing", say *what*
you're about to enumerate. The intro sentence is the table of contents for the
next 3–5 paragraphs.

```text
✅ When hot reload is on, SweetPad does three things to your build and launch
   environment.

   **Build flags.** It appends …
   **Launch env.** It sets …
   **Framework paths.** It prepends …
```

Without the framing sentence the reader has to assemble the structure on the
fly. With it, they know exactly what they're navigating.

## 5. Headings beat numbered lists for navigation

A six-step list with multi-paragraph items is unscannable. Promote each step
to a `###` heading. The TOC in the sidebar then mirrors the structure and the
reader can jump.

Reserve numbered lists for things that are genuinely short and ordered (a quick
recipe, a five-step "try it" walkthrough).

## 6. Use admonitions for callouts, not bold paragraphs

Docusaurus has `:::tip`, `:::info`, `:::warning`, `:::danger`, `:::note`. Use
them. They give the eye an anchor, they're styled consistently, and they're
better than a paragraph of bold text trying to do the same job.

```markdown
:::tip
You only need this on views you actively edit. A top-level view and the ones
you're iterating on are usually enough.
:::
```

## 7. The visual default is subtle

The site's inline-`code` styling is a soft gray fill with no border, ~90% of
body text size. It's defined in `src/css/custom.css` via the Infima variables
`--ifm-code-background`, `--ifm-code-padding-*`, `--ifm-code-border-radius`,
plus a `code { border: none }` override.

If you ever feel tempted to add a more prominent style for "important code",
don't — write a clearer sentence instead. Loud styling is a band-aid for
prose that doesn't carry its weight.

---

## Quick checklist

Before pushing a doc edit, ask:

- Am I code-formatting any *concept*? Strip those backticks.
- Am I citing a specific source file or function? Restate conceptually.
- Is any paragraph longer than ~10 lines? Split it.
- Is any numbered-list item longer than ~3 sentences? Promote to a heading.
- Is there a 3-things-happen paragraph without an intro sentence? Add one.
- Am I using bold text where an admonition would do?

If everything answers "no", ship it.

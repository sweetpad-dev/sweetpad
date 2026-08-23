# SweetPad documentation website

The documentation site for both SweetPad products — the `sweetpad` CLI and the VS Code extension —
built with [Docusaurus](https://docusaurus.io/). The pages themselves are Markdown files in `docs`.

Each product has its own folder, its own sidebar, and its own URL namespace, so a page is never
ambiguous about which tool it describes:

- `docs/cli/` → `/docs/cli/…`, the **CLI** sidebar
- `docs/vscode/` → `/docs/vscode/…`, the **VS Code** sidebar
- `docs/intro.md` → `/docs`, the "which one do I need?" page shown in both sidebars
- `docs/contributing/` → developing and releasing SweetPad itself

Do not add a `slug:` to a page's frontmatter — the folder is what puts the product in the URL. When a
page moves or is renamed, add a redirect from its old path in `docusaurus.config.ts`.

Both sidebars are listed by hand in `sidebars.ts` so their pages can be grouped, so a new page needs
a line there as well as its file.

[STYLE.md](./STYLE.md) covers how to write a page.

I'm open to contributions to this documentation 🤝. Here is the official
**[GitHub documentation](https://docs.github.com/en/get-started/exploring-projects-on-github/contributing-to-a-project)**
how to contribute to projects on GitHub.

# Installation

```
npm install
```

# Development

```bash
npm run start
```

Open [http://localhost:3000](http://localhost:3000) to view it in the browser.

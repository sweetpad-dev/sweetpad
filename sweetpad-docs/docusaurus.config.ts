import { themes as prismThemes } from 'prism-react-renderer';
import type { Config } from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

/**
 * Docs used to live at flat, product-less paths (`/docs/build`, `/docs/cli`).
 * They now sit under `/docs/cli/…` and `/docs/vscode/…` so the URL says which
 * product a page belongs to. Every old path redirects to its new home.
 */
const cliRedirects: Record<string, string> = {
  '/docs/getting-started-cli': '/docs/cli/getting-started',
  '/docs/cli': '/docs/cli/getting-started',
  '/docs/cli-reference': '/docs/cli/reference',
  '/docs/agent-cli': '/docs/cli/agent-cli',
  '/docs/agent-skills': '/docs/cli/agent-skills',
  '/docs/category/cli': '/docs/cli/getting-started',
};

const vscodeRedirects: Record<string, string> = {
  '/docs/getting-started-vscode': '/docs/vscode/getting-started',
  '/docs/build': '/docs/vscode/build',
  '/docs/debug': '/docs/vscode/debug',
  '/docs/hot-reload': '/docs/vscode/hot-reload',
  '/docs/tests': '/docs/vscode/tests',
  '/docs/format': '/docs/vscode/format',
  '/docs/autocomplete': '/docs/vscode/autocomplete',
  '/docs/destinations': '/docs/vscode/destinations',
  '/docs/simulators': '/docs/vscode/simulators',
  '/docs/watchos-simulators': '/docs/vscode/simulators#watchos-simulators',
  '/docs/devices': '/docs/vscode/devices',
  '/docs/tools': '/docs/vscode/tools',
  '/docs/tuist': '/docs/vscode/tuist',
  '/docs/xcodegen': '/docs/vscode/xcodegen',
  '/docs/worktree': '/docs/vscode/worktree',
  '/docs/settings': '/docs/vscode/settings',
  '/docs/commands': '/docs/vscode/commands',
  '/docs/troubleshooting': '/docs/vscode/troubleshooting',
  '/docs/category/vscode-extension': '/docs/vscode/getting-started',
};

/** Pages that were folded into another page, keyed by the URL they used to own. */
const foldedPages: Record<string, string> = {
  '/docs/vscode/watchos-simulators': '/docs/vscode/simulators#watchos-simulators',
};

const redirects = Object.entries({
  ...cliRedirects,
  ...vscodeRedirects,
  ...foldedPages,
  '/docs/intro': '/docs',
}).map(([from, to]) => ({ from, to }));

const config: Config = {
  title: 'SweetPad',
  tagline: 'Build, run, and test Xcode apps from the terminal — or from VS Code',
  favicon: 'images/favicon.ico',

  // Set the production url of your site here
  url: 'https://sweetpad.hyzyla.dev',
  // Set the /<baseUrl>/ pathname under which your site is served
  // For GitHub pages deployment, it is often '/<projectName>/'
  baseUrl: '/',

  // GitHub pages deployment config.
  // If you aren't using GitHub pages, you don't need these.
  organizationName: 'github', // Usually your GitHub org/user name.
  projectName: 'docusaurus', // Usually your repo name.

  onBrokenLinks: 'throw',

  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'warn',
    },
  },
  themes: [
    '@docusaurus/theme-mermaid',
    [
      // Offline search: the index is built at compile time and served as a
      // static asset, so there's no crawler and no third-party service.
      '@easyops-cn/docusaurus-search-local',
      {
        hashed: true,
        indexDocs: true,
        indexBlog: true,
        indexPages: false,
        docsRouteBasePath: '/docs',
        language: ['en'],
        highlightSearchTermsOnTargetPage: true,
        // The two products have pages with the same names — Simulators,
        // Destinations, Troubleshooting. Showing each hit's path is what tells
        // a CLI result apart from a VS Code one.
        explicitSearchResultPath: true,
        searchResultLimits: 10,
      },
    ],
  ],

  // Even if you don't use internationalization, you can use this field to set
  // useful metadata like html lang. For example, if your site is Chinese, you
  // may want to replace "en" with "zh-Hans".
  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  plugins: [
    [
      "posthog-docusaurus",
      {
        apiKey: "phc_D1i4iVeA9TL2yR833rVrirR6aQILA6eu601xcPNGD9k",
        appUrl: "https://eu.i.posthog.com",
        enableInDevelopment: true,
      },
    ],
    [
      "@docusaurus/plugin-client-redirects",
      {
        redirects,
      },
    ],
  ],

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          // Please change this to your repo.
          // Remove this to remove the "edit this page" links.
          editUrl:
            'https://github.com/sweetpad-dev/sweetpad/edit/main/sweetpad-docs/',
        },
        // blog: {
        //   showReadingTime: true,
        //   feedOptions: {
        //     type: ['rss', 'atom'],
        //     xslt: true,
        //   },
        //   // Please change this to your repo.
        //   // Remove this to remove the "edit this page" links.
        //   editUrl:
        //     'https://github.com/facebook/docusaurus/tree/main/packages/create-docusaurus/templates/shared/',
        //   // Useful options to enforce blogging best practices
        //   onInlineTags: 'warn',
        //   onInlineAuthors: 'warn',
        //   onUntruncatedBlogPosts: 'warn',
        // },
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    // Replace with your project's social card
    image: 'images/logo.png',
    navbar: {
      title: 'SweetPad',
      logo: {
        alt: 'SweetPad logo',
        src: 'images/logo.svg',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'cliSidebar',
          position: 'left',
          label: 'CLI',
        },
        {
          type: 'docSidebar',
          sidebarId: 'vscodeSidebar',
          position: 'left',
          label: 'VS Code',
        },
        { to: '/blog', label: 'Blog', position: 'left' },
        {
          href: 'https://github.com/sweetpad-dev/sweetpad',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'SweetPad CLI',
          items: [
            {
              label: 'Get started',
              to: '/docs/cli/getting-started',
            },
            {
              label: 'Command reference',
              to: '/docs/cli/reference',
            },
            {
              label: 'Agent skills',
              to: '/docs/cli/agent-skills',
            },
          ],
        },
        {
          title: 'VS Code extension',
          items: [
            {
              label: 'Get started',
              to: '/docs/vscode/getting-started',
            },
            {
              label: 'VS Code Marketplace',
              href: 'https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad',
            },
          ],
        },
        {
          title: 'More',
          items: [
            {
              label: 'Which one do I need?',
              to: '/docs',
            },
            {
              label: 'Blog',
              to: '/blog',
            },
            {
              label: 'GitHub',
              href: 'https://github.com/sweetpad-dev/sweetpad',
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Yevhenii Hyzyla. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['json', 'bash'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;

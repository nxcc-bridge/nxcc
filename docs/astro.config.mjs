// @ts-check
import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";

import tailwindcss from "@tailwindcss/vite";

import vue from "@astrojs/vue";

// https://astro.build/config
export default defineConfig({
  // Canonical site URL for sitemap and absolute links.
  site: "https://nxcc.org",
  integrations: [
    starlight({
      title: "nXCC",
      logo: {
        src: "./public/logo.png",
      },
      expressiveCode: {
        themes: ["everforest-light", "everforest-dark"],
        defaultProps: {
          overridesByLang: {
            "bash,sh,zsh,shell,shellsession,console": { frame: "code" },
          },
        },
        styleOverrides: {
          frames: {
            frameBoxShadowCssValue: "none",
            terminalTitlebarDotsOpacity: "0",
          },
        },
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/nxcc-bridge/nxcc",
        },
      ],
      sidebar: [
        {
          label: "Guides",
          items: [
            { label: "Getting Started", slug: "docs/guides/getting-started" },
            {
              label: "Blockchain Events",
              slug: "docs/guides/blockchain-events",
            },
            {
              label: "Identities & Policies",
              slug: "docs/guides/identities-policies",
            },
            {
              label: "Zenroom Interop",
              slug: "docs/guides/zenroom-vm",
            },
          ],
        },
        {
          label: "Reference",
          items: [
            { label: "Reference", slug: "docs/reference" },
            {
              label: "Node Operators",
              items: [
                {
                  label: "Running a Node",
                  slug: "docs/reference/node-operators/running-a-node",
                },
                {
                  label: "Infrastructure Management",
                  slug: "docs/reference/node-operators/infra-management",
                },
                {
                  label: "Performance & Efficiency",
                  slug: "docs/reference/node-operators/performance",
                },
              ],
            },
            {
              label: "Developers",
              items: [
                {
                  label: "Core Concepts",
                  slug: "docs/reference/developers/core-concepts",
                },
                {
                  label: "CLI Reference",
                  slug: "docs/reference/developers/cli",
                },
                {
                  label: "SDK Reference",
                  slug: "docs/reference/developers/sdk-reference",
                },
                {
                  label: "Worker Manifest Reference",
                  slug: "docs/reference/developers/worker-manifest",
                },
                {
                  label: "Worker Runtime APIs",
                  slug: "docs/reference/developers/worker-runtime",
                },
                {
                  label: "Event Triggers",
                  slug: "docs/reference/developers/event-triggers",
                },
                {
                  label: "Identities & Policies",
                  slug: "docs/reference/developers/identities-and-policies",
                },
                {
                  label: "Zenroom VM",
                  slug: "docs/reference/developers/zenroom-vm",
                },
              ],
            },
          ],
        },
      ],
      components: {
        ThemeProvider: "./src/components/ThemeProvider.astro",
        ThemeSelect: "./src/components/ThemeSelect.astro",
      },
      customCss: ["./src/styles/global.css"],
    }),
    vue(),
    sitemap(),
  ],

  vite: {
    plugins: [tailwindcss()],
  },
});

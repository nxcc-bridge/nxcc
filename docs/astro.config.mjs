// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

import tailwindcss from "@tailwindcss/vite";

import vue from "@astrojs/vue";

// https://astro.build/config
export default defineConfig({
  integrations: [
    starlight({
      title: "nXCC",
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
          ],
        },
        {
          label: "Reference",
          autogenerate: { directory: "docs/reference" },
        },
      ],
      customCss: ["./src/styles/global.css"],
    }),
    vue(),
  ],

  vite: {
    plugins: [tailwindcss()],
  },
});

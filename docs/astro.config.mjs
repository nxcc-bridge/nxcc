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
            // Each item here is one entry in the navigation menu.
            { label: "Getting Started", slug: "docs/guides/getting-started" },
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

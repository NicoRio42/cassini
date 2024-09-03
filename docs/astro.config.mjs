import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import rehypeExternalLinks from "rehype-external-links";

// https://astro.build/config
export default defineConfig({
  integrations: [
    starlight({
      title: "Cassini",
      logo: {
        dark: "./src/assets/logo-dark.svg",
        light: "./src/assets/logo-light.svg",
      },
      social: {
        github: "https://github.com/NicoRio42/cassini",
      },
      sidebar: [
        { label: "The what and the why", link: "/what-and-why" },
        {
          label: "Guides",
          items: [
            {
              label: "Installation and setup",
              slug: "guides/installation-and-setup",
            },
            {
              label: "Process a single LiDAR file",
              slug: "guides/single-tile",
            },
            { label: "Batch mode", slug: "guides/batch-mode" },
            { label: "Vector files", slug: "guides/vector-files" },
          ],
        },
        {
          label: "Reference",
          items: [
            {
              label: "CLI Reference",
              slug: "reference/cli-reference",
            },
            {
              label: "Configuration Reference",
              slug: "reference/configuration-reference",
            },
            { label: "Glossary", slug: "reference/glossary" },
          ],
        },
      ],
      favicon: "./src/assets/favicon.ico",
      components: {
        PageSidebar: "./src/components/PageSidebar.astro",
      },
      customCss: ["./src/styles/custom.css"],
    }),
  ],
  markdown: { rehypePlugins: [[rehypeExternalLinks, { target: "_blank" }]] },
});

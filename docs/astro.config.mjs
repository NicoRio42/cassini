import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

// https://astro.build/config
export default defineConfig({
  integrations: [
    starlight({
      title: "Cassini",
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
          autogenerate: { directory: "reference" },
        },
      ],
    }),
  ],
});

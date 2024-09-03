import { defineCollection, z } from "astro:content";
import { docsSchema } from "@astrojs/starlight/schema";

export const collections = {
  docs: defineCollection({ schema: docsSchema() }),
  glossary: defineCollection({
    type: "content",
    schema: z.object({
      defaultLabel: z.string(),
      externalUrl: z.string(),
    }),
  }),
};

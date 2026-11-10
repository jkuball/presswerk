import { defineCollection } from 'astro:content';
import { docsSchema } from '@astrojs/starlight/schema';
import { docsWithReferenceLoader, optionsLoader } from './reference/loaders';

export const collections = {
  docs: defineCollection({ loader: docsWithReferenceLoader(), schema: docsSchema() }),
  options: defineCollection({ loader: optionsLoader() }),
};

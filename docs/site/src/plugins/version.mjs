import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const placeholder = /\{\{\s*presswerk\.version\s*\}\}/g;

/**
 * Sätteri mdast plugin: replace {{presswerk.version}} with the version these docs describe.
 *
 * The version comes from the generated reference data, so it always matches the
 * presswerk.default.version of the commit the site is built from. Code blocks and
 * inline code are covered as well as prose, so examples can pin that release.
 */
export default function versionPlugin() {
  let version;
  const substitute = (node) => {
    if (!node.value.match(placeholder)) return;
    version ??= readVersion();
    return { ...node, value: node.value.replace(placeholder, version) };
  };

  return { name: 'presswerk-version', text: substitute, inlineCode: substitute, code: substitute };
}

function readVersion() {
  const directory = process.env.PRESSWERK_REFERENCE;
  if (!directory) {
    throw new Error('PRESSWERK_REFERENCE is not set; it provides the version for {{presswerk.version}}.');
  }
  return readFileSync(join(directory, 'version'), 'utf8').trim();
}

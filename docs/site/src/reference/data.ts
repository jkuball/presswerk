import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * Generated reference data, built by the `docs-reference` flake package and passed in through
 * $PRESSWERK_REFERENCE. Every value here comes from the module, the CLI, or the catalog itself.
 */

export interface Literal {
  kind: 'nix' | 'markdown';
  text: string;
}

export interface ReleaseOption {
  name: string;
  loc: string[];
  anchor: string;
  type: string;
  description: string;
  readOnly: boolean;
  default: Literal | null;
  example: Literal | null;
  declarations: { path: string; line: number; url: string }[];
}

export interface CliArgument {
  id: string;
  long: string | null;
  short: string | null;
  valueNames: string[];
  required: boolean;
  defaults: string[];
  help: string | null;
}

export interface CliCommand {
  name: string;
  about: string | null;
  usage: string;
  arguments: CliArgument[];
  options: CliArgument[];
  subcommands: CliCommand[];
}

export interface GitmojiEntry {
  emoji: string;
  description: string;
  semver: 'major' | 'minor' | 'patch' | null;
}

export interface ConventionalEntry {
  title: string;
  semver: 'major' | 'minor' | 'patch' | null;
}

export interface ReferenceData {
  options: ReleaseOption[];
  cli: CliCommand;
  declaration: Record<string, unknown>;
  gitmoji: Record<string, GitmojiEntry>;
  /** The default Conventional Commits type catalog. */
  conventional: Record<string, ConventionalEntry>;
  /** The upstream MIT notice of the vendored Gitmoji catalog. */
  gitmojiLicense: string;
  version: string;
}

let cached: ReferenceData | undefined;

export function referenceData(): ReferenceData {
  if (cached) return cached;
  const directory = process.env.PRESSWERK_REFERENCE;
  if (!directory) {
    throw new Error(
      'PRESSWERK_REFERENCE is not set; build the reference data with `nix build .#docs-reference` ' +
        'and point it at the result, or use the `docs-preview` devshell command.',
    );
  }
  const read = (file: string) => JSON.parse(readFileSync(join(directory, file), 'utf8'));
  cached = {
    options: read('options.json'),
    cli: read('cli.json'),
    declaration: read('declaration.json'),
    gitmoji: read('gitmoji.json'),
    conventional: read('conventional.json'),
    gitmojiLicense: readFileSync(join(directory, 'gitmoji-license.txt'), 'utf8'),
    version: readFileSync(join(directory, 'version'), 'utf8').trim(),
  };
  return cached;
}

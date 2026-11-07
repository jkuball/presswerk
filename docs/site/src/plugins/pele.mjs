import { render } from 'pele';
import { htmlToHast } from 'satteri';

/**
 * Sätteri mdast plugin: render ```mermaid code blocks to inline SVG at build time with Pele.
 *
 * The SVG takes its colours and fonts from the --pele-* custom properties in
 * src/styles/presswerk.css, so diagrams follow the light and dark theme without
 * client-side JavaScript.
 */
export default function pelePlugin() {
  return {
    name: 'presswerk-pele',
    code(node) {
      if (node.lang !== 'mermaid') return;
      const { svg } = render(node.value);
      return {
        type: 'paragraph',
        data: {
          hName: 'figure',
          hProperties: { className: ['pw-graph', 'not-content'] },
          hChildren: htmlToHast(svg, { fragment: true }).children,
        },
        children: [],
      };
    },
  };
}

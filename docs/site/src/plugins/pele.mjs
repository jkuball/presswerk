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
  let figureNumber = 0;

  return {
    name: 'presswerk-pele',
    code(node) {
      if (node.lang !== 'mermaid') return;

      const caption = node.meta?.trim();
      if (!caption) throw new Error('Mermaid diagrams need a caption after the language name.');

      const captionId = `pw-graph-${++figureNumber}-caption`;
      const { svg } = render(node.value);
      const diagram = htmlToHast(svg, { fragment: true }).children;
      const svgNode = diagram.find((child) => child.type === 'element' && child.tagName === 'svg');
      if (svgNode) svgNode.properties.ariaLabelledBy = captionId;

      return {
        type: 'paragraph',
        data: {
          hName: 'figure',
          hProperties: { className: ['pw-graph', 'not-content'], ariaLabelledBy: captionId },
          hChildren: [
            ...diagram,
            {
              type: 'element',
              tagName: 'figcaption',
              properties: { id: captionId },
              children: [{ type: 'text', value: caption }],
            },
          ],
        },
        children: [],
      };
    },
  };
}

import { defineRouteMiddleware, type StarlightRouteData } from '@astrojs/starlight/route-data';

type SidebarEntry = StarlightRouteData['sidebar'][number];

/** The label of the nested sidebar group that lists `href` directly, if any. */
function nestedGroupOf(entries: SidebarEntry[], href: string, depth = 0): string | undefined {
  for (const entry of entries) {
    if (entry.type !== 'group') continue;
    if (depth > 0 && entry.entries.some((child) => child.type === 'link' && child.href === href)) return entry.label;
    const label = nestedGroupOf(entry.entries, href, depth + 1);
    if (label) return label;
  }
}

/** A pagination label that names the group, for sidebar labels that only make sense under it. */
function labelWithGroup(label: string, group: string): string | undefined {
  if (label === 'Overview') return group;
  // "Commit analyzers" → "Build your own commit analyzer"
  if (label === 'Build your own') return `${label} ${group.toLowerCase().replace(/s$/, '')}`;
}

// Nested groups label their pages "Overview" and "Build your own" in the sidebar, under the group's
// name. Previous and next links show those labels without the group, so they name it themselves.
export const onRequest = defineRouteMiddleware((context) => {
  const { sidebar, pagination } = context.locals.starlightRoute;
  for (const key of ['prev', 'next'] as const) {
    const link = pagination[key];
    if (!link) continue;
    const group = nestedGroupOf(sidebar, link.href);
    const label = group && labelWithGroup(link.label, group);
    if (label) pagination[key] = { ...link, label };
  }
});

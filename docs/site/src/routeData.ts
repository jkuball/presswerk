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

// The landing page of a nested group is labelled "Overview" in the sidebar, under the group's name.
// Previous and next links show that label without the group, so they take the group's name instead.
export const onRequest = defineRouteMiddleware((context) => {
  const { sidebar, pagination } = context.locals.starlightRoute;
  for (const key of ['prev', 'next'] as const) {
    const link = pagination[key];
    if (link?.label !== 'Overview') continue;
    const group = nestedGroupOf(sidebar, link.href);
    if (group) pagination[key] = { ...link, label: group };
  }
});

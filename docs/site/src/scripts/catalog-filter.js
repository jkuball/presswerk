/**
 * Turn the default Gitmoji catalog on the commit analyzers page into a filterable grid. The page
 * itself stays plain Markdown lists, so it reads the same without JavaScript and as plain text.
 */
const heading = document.getElementById('default-gitmoji-catalog');
if (heading) {
  const section = [];
  for (let node = heading.closest('.sl-heading-wrapper') ?? heading; (node = node.nextElementSibling); ) {
    if (node.matches('h2, .sl-heading-wrapper:has(h2)')) break;
    section.push(node);
  }
  const lists = section.filter((node) => node.matches('ul'));
  // Variation selector-16 is optional in commits, so searching for an emoji ignores it too.
  const normalize = (text) => text.toLowerCase().replaceAll('️', '');
  const entries = lists.flatMap((list) => {
    list.classList.add('pw-catalog');
    return [...list.children].map((item) => {
      const code = item.querySelector('code');
      const emoji = item.firstChild.textContent.trim();
      const description = item.textContent.slice(item.textContent.indexOf(code.textContent) + code.textContent.length).trim();
      item.replaceChildren(
        Object.assign(document.createElement('span'), { className: 'pw-catalog-emoji', textContent: emoji }),
        code,
        Object.assign(document.createElement('span'), { className: 'pw-catalog-description', textContent: description }),
      );
      return { item, list, text: normalize(`${emoji} ${code.textContent} ${description}`) };
    });
  });

  const filter = document.createElement('div');
  filter.className = 'pw-filter not-content';
  filter.innerHTML =
    '<label for="pw-catalog-filter">Filter the catalog</label>' +
    '<input id="pw-catalog-filter" type="search" placeholder="bug, docs, 🔥…" autocomplete="off" />' +
    '<output for="pw-catalog-filter" aria-live="polite"></output>';
  const input = filter.querySelector('input');
  const status = filter.querySelector('output');
  section.find((node) => node.matches('h3, .sl-heading-wrapper:has(h3)'))?.before(filter);

  input.addEventListener('input', () => {
    const terms = normalize(input.value).split(/\s+/).filter(Boolean);
    let shown = 0;
    for (const entry of entries) {
      entry.item.hidden = !terms.every((term) => entry.text.includes(term));
      if (!entry.item.hidden) shown += 1;
    }
    // Hide a level whose entries are all filtered out, together with its heading.
    for (const list of lists) {
      const empty = [...list.children].every((item) => item.hidden);
      // Starlight lays out heading wrappers with display, which overrides the hidden attribute.
      for (const node of [list, list.previousElementSibling]) if (node) node.style.display = empty ? 'none' : '';
    }
    status.value = !terms.length
      ? ''
      : shown
        ? `${shown} of ${entries.length} entries`
        : 'No entries match; clear the filter to show all.';
  });
}

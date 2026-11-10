/**
 * Add a filter above every list of release options: on the options page and in the option lists of
 * the reference pages. Without JavaScript the lists show every option.
 */
document.querySelectorAll('.pw-option-list').forEach((list, index) => {
  const sections = [...list.querySelectorAll(':scope > .pw-option')];
  const id = `pw-option-filter-${index}`;
  const filter = document.createElement('div');
  filter.className = 'pw-filter not-content';
  filter.innerHTML =
    `<label for="${id}">Filter options</label>` +
    `<input id="${id}" type="search" autocomplete="off" />` +
    `<output for="${id}" aria-live="polite"></output>`;
  const input = filter.querySelector('input');
  const status = filter.querySelector('output');
  input.placeholder = list.dataset.placeholder ?? 'name or description…';
  list.before(filter);

  function apply() {
    const terms = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    let shown = 0;
    for (const section of sections) {
      const match = terms.every((term) => section.dataset.option.includes(term));
      section.hidden = !match;
      if (match) shown += 1;
    }
    status.value = !terms.length
      ? ''
      : shown
        ? `${shown} of ${sections.length} options`
        : 'No options match; clear the filter to show all.';
  }

  input.addEventListener('input', apply);
  // Never hide the option a link points at.
  window.addEventListener('hashchange', () => {
    const target = document.getElementById(decodeURIComponent(location.hash.slice(1)));
    if (target?.closest('.pw-option')?.hidden && list.contains(target)) {
      input.value = '';
      apply();
      target.scrollIntoView();
    }
  });
});

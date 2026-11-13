/**
 * Add a button that clears the input to every filter added by src/scripts/option-filter.js and
 * src/scripts/catalog-filter.js. It shows only while the filter has text.
 */
document.querySelectorAll('.pw-filter input').forEach((input) => {
  const field = document.createElement('span');
  field.className = 'pw-filter-field';
  const clear = document.createElement('button');
  clear.type = 'button';
  clear.className = 'pw-filter-clear';
  clear.setAttribute('aria-label', 'Clear the filter');
  clear.textContent = '×';
  input.replaceWith(field);
  field.append(input, clear);

  const update = () => (clear.hidden = !input.value);
  input.addEventListener('input', update);
  clear.addEventListener('click', () => {
    input.value = '';
    input.dispatchEvent(new Event('input'));
    input.focus();
  });
  update();
});

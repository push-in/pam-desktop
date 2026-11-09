// Collects the boxes a layout comparison needs from a Chromium page:
// every element that holds text of its own (its direct text nodes) with
// getBoundingClientRect() in CSS px. Evaluate in the page; it returns JSON.
// Used by electron-dom.cjs and usable from Playwright/Puppeteer as well.
(() => {
  const out = [];
  for (const element of document.querySelectorAll('body *')) {
    const own = [...element.childNodes]
      .filter((node) => node.nodeType === Node.TEXT_NODE)
      .map((node) => node.textContent)
      .join('')
      .replace(/\s+/g, ' ')
      .trim();
    if (!own) continue;
    const style = getComputedStyle(element);
    if (style.visibility === 'hidden' || style.display === 'none') continue;
    const rect = element.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) continue;
    out.push({
      tag: element.tagName.toLowerCase(),
      className: String(element.className && element.className.baseVal !== undefined ? element.className.baseVal : element.className || ''),
      // innerText applies text-transform like the native text does.
      text: (element.innerText || own).replace(/\s+/g, ' ').trim().slice(0, 120),
      ownText: own.slice(0, 120),
      rect: [rect.x, rect.y, rect.width, rect.height],
      // Computed text metrics and border widths (left, top, right, bottom)
      // a native port declares: PrimeVue labels inherit line-height 1.5 and
      // text buttons keep a transparent 1px border.
      font: [style.fontSize, style.lineHeight, style.fontWeight],
      border: ['borderLeftWidth', 'borderTopWidth', 'borderRightWidth', 'borderBottomWidth'].map((name) => style[name]),
      padding: ['paddingLeft', 'paddingTop', 'paddingRight', 'paddingBottom'].map((name) => style[name]),
      // The button a label belongs to (its own box has no text): rect,
      // border and padding widths and line-height.
      ...(() => {
        const control = element.closest('button,[role=button]');
        if (!control || control === element) return {};
        const cs = getComputedStyle(control);
        const r = control.getBoundingClientRect();
        return { control: { rect: [r.x, r.y, r.width, r.height], border: [cs.borderLeftWidth, cs.borderTopWidth, cs.borderRightWidth, cs.borderBottomWidth], padding: [cs.paddingLeft, cs.paddingTop, cs.paddingRight, cs.paddingBottom], font: [cs.fontSize, cs.lineHeight] } };
      })(),
      // The nearest compositing layer (will-change: transform) holding the
      // text: content inside it is laid out from its snapped origin.
      ...(() => {
        for (let node = element.parentElement; node; node = node.parentElement) {
          if (getComputedStyle(node).willChange.includes('transform')) {
            const r = node.getBoundingClientRect();
            return { layer: [r.x, r.y, r.width, r.height] };
          }
        }
        return {};
      })(),
      // Computed corner radii (top-left, top-right, bottom-right, bottom-left)
      // when the box is rounded, so a native port declares the same value.
      ...(() => {
        const radii = ['borderTopLeftRadius', 'borderTopRightRadius', 'borderBottomRightRadius', 'borderBottomLeftRadius'].map((name) => style[name]);
        return radii.some((value) => parseFloat(value) > 0) ? { radius: radii } : {};
      })(),
      // The content box (padding and border removed), what a native Text
      // without padding of its own is laid out as.
      content: (() => {
        const px = (name) => parseFloat(style[name]) || 0;
        // A cell of a collapsing table holds half of each border (CSS 2.1
        // 17.6.2); getComputedStyle reports the authored width.
        const table = /^table-cell$/.test(style.display) && element.closest('table');
        const share = table && getComputedStyle(table).borderCollapse === 'collapse' ? 0.5 : 1;
        const left = px('borderLeftWidth') * share + px('paddingLeft');
        const top = px('borderTopWidth') * share + px('paddingTop');
        const right = px('borderRightWidth') * share + px('paddingRight');
        const bottom = px('borderBottomWidth') * share + px('paddingBottom');
        return [rect.x + left, rect.y + top, Math.max(0, rect.width - left - right), Math.max(0, rect.height - top - bottom)];
      })(),
    });
  }
  return JSON.stringify({ width: innerWidth, height: innerHeight, devicePixelRatio, nodes: out });
})();

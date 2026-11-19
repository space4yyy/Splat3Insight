(() => {
  'use strict';

  let supportsDisplayTransition;
  function canTransitionDisplay() {
    if (supportsDisplayTransition !== undefined) return supportsDisplayTransition;
    if (!window.CSS?.supports?.('transition-behavior', 'allow-discrete') || !document.body) {
      return false;
    }
    const probe = document.createElement('div');
    probe.style.cssText = 'transition: display 1s allow-discrete !important; display: block;';
    document.body.appendChild(probe);
    getComputedStyle(probe).display;
    probe.style.display = 'none';
    supportsDisplayTransition = getComputedStyle(probe).display === 'block';
    probe.remove();
    return supportsDisplayTransition;
  }

  function supportsTopLayerExit() {
    return Boolean(window.CSS?.supports?.('overlay', 'auto') && canTransitionDisplay());
  }

  function showElement(el) {
    if (!el) return;
    el.removeAttribute('data-closing');
    el.hidden = false;
  }

  async function hideElement(el) {
    if (!el || el.hidden) return;
    if (canTransitionDisplay()) {
      el.hidden = true;
      return;
    }
    el.setAttribute('data-closing', '');
    const animations = el.getAnimations();
    if (animations.length > 0) {
      await Promise.race([
        Promise.allSettled(animations.map((a) => a.finished)),
        new Promise((r) => setTimeout(r, 2000)),
      ]);
    }
    if (el.hasAttribute('data-closing')) {
      el.removeAttribute('data-closing');
      el.hidden = true;
    }
  }

  async function closeDialog(dialog) {
    if (!dialog || !dialog.open) return;
    if (!supportsTopLayerExit()) {
      if (dialog.hasAttribute('data-closing')) return;
      dialog.setAttribute('data-closing', '');
      const animations = dialog.getAnimations({ subtree: true });
      if (animations.length > 0) {
        await Promise.race([
          Promise.allSettled(animations.map((a) => a.finished)),
          new Promise((r) => setTimeout(r, 2000)),
        ]);
      }
      if (!dialog.hasAttribute('data-closing')) return;
      dialog.removeAttribute('data-closing');
    }
    if (dialog.open) dialog.close();
  }

  function setupDialog(dialog, { canClose, lightDismiss = true } = {}) {
    if (!dialog) return;
    dialog.addEventListener('cancel', (event) => {
      event.preventDefault();
      if (canClose && !canClose()) return;
      void closeDialog(dialog);
    });
    if (lightDismiss && !('closedBy' in HTMLDialogElement.prototype)) {
      let pressedOutside = false;
      const isOutside = (event) => {
        if (event.target !== dialog) return false;
        const rect = dialog.getBoundingClientRect();
        return !(
          rect.top <= event.clientY &&
          event.clientY <= rect.top + rect.height &&
          rect.left <= event.clientX &&
          event.clientX <= rect.left + rect.width
        );
      };
      dialog.addEventListener('pointerdown', (event) => {
        pressedOutside = isOutside(event);
      });
      dialog.addEventListener('click', (event) => {
        if (pressedOutside && isOutside(event) && (!canClose || canClose())) {
          void closeDialog(dialog);
        }
        pressedOutside = false;
      });
    }
  }

  const supportsSiblingIndex = Boolean(window.CSS?.supports?.('animation-delay: calc(sibling-index() * 0.1s)'));
  function applySiblingStagger(root = document) {
    if (supportsSiblingIndex || !root) return;
    for (const container of root.querySelectorAll('.kpis, .grid')) {
      [...container.children].forEach((child, index) => {
        child.style.setProperty('--sibling-index', String(index + 1));
      });
    }
  }

  const reducedMotionQuery = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  function runViewTransition(update, onFinish, { type = 'nav' } = {}) {
    if (document.startViewTransition && !reducedMotionQuery?.matches) {
      document.documentElement.dataset.vt = type;
      const transition = document.startViewTransition(update);
      transition.finished.finally(() => {
        if (document.documentElement.dataset.vt === type) {
          delete document.documentElement.dataset.vt;
        }
        if (onFinish) onFinish();
      });
      return transition;
    }
    update();
    if (onFinish) onFinish();
    return null;
  }

  // 分段控件滑动胶囊：支持原生 CSS Anchor Positioning 时由纯 CSS 驱动，否则用轻量的变量回退。
  const supportsAnchorScope = Boolean(
    window.CSS?.supports?.('anchor-scope', '--seg-active')
    && window.CSS?.supports?.('position-anchor', '--seg-active')
    && window.CSS?.supports?.('inset', 'anchor(top) anchor(right) anchor(bottom) anchor(left)')
  );
  const trackedSegmented = new WeakSet();
  let segmentedResizeObserver = null;

  function updateSegmentedIndicator(container, { immediate = false } = {}) {
    if (!container || supportsAnchorScope) return;
    const active = container.querySelector('button[aria-pressed="true"]');
    const rect = container.getBoundingClientRect();
    if (!active || rect.width === 0 || rect.height === 0) {
      container.style.setProperty('--seg-opacity', '0');
      return;
    }
    const buttonRect = active.getBoundingClientRect();
    if (buttonRect.width === 0 || buttonRect.height === 0) {
      container.style.setProperty('--seg-opacity', '0');
      return;
    }
    const x = buttonRect.left - rect.left - container.clientLeft;
    const y = buttonRect.top - rect.top - container.clientTop;
    if (immediate) container.classList.remove('is-ready');
    container.style.setProperty('--seg-x', `${x.toFixed(1)}px`);
    container.style.setProperty('--seg-y', `${y.toFixed(1)}px`);
    container.style.setProperty('--seg-w', `${buttonRect.width.toFixed(1)}px`);
    container.style.setProperty('--seg-h', `${buttonRect.height.toFixed(1)}px`);
    container.style.setProperty('--seg-opacity', '1');
    if (!container.classList.contains('is-ready')) {
      requestAnimationFrame(() => {
        if (container.isConnected) container.classList.add('is-ready');
      });
    }
  }

  function observeSegmented(container) {
    if (!container || supportsAnchorScope || trackedSegmented.has(container)) return;
    trackedSegmented.add(container);
    container.classList.add('has-indicator');
    updateSegmentedIndicator(container, { immediate: true });
    new MutationObserver(() => updateSegmentedIndicator(container)).observe(container, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['aria-pressed', 'hidden'],
    });
    if (segmentedResizeObserver) segmentedResizeObserver.observe(container);
  }

  function syncSegmentedIndicators(root = document, { immediate = false } = {}) {
    if (supportsAnchorScope || !root) return;
    for (const container of root.querySelectorAll('.segmented')) {
      if (!trackedSegmented.has(container)) observeSegmented(container);
      else updateSegmentedIndicator(container, { immediate });
    }
  }

  function initSegmentedIndicators() {
    if (supportsAnchorScope) return;
    if (typeof ResizeObserver === 'function') {
      segmentedResizeObserver = new ResizeObserver((entries) => {
        for (const entry of entries) {
          updateSegmentedIndicator(entry.target, { immediate: !entry.target.classList.contains('is-ready') });
        }
      });
    }
    syncSegmentedIndicators(document, { immediate: true });
    document.fonts?.ready.then(() => syncSegmentedIndicators(document, { immediate: true }));
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initSegmentedIndicators, { once: true });
  } else {
    initSegmentedIndicators();
  }

  window.Splat3Motion = Object.freeze({
    canTransitionDisplay,
    supportsTopLayerExit,
    showElement,
    hideElement,
    closeDialog,
    setupDialog,
    applySiblingStagger,
    runViewTransition,
    syncSegmentedIndicators,
  });
})();

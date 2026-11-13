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
  function runViewTransition(update, onFinish) {
    if (document.startViewTransition && !reducedMotionQuery?.matches) {
      const transition = document.startViewTransition(update);
      if (onFinish) transition.finished.finally(onFinish);
      return transition;
    }
    update();
    if (onFinish) onFinish();
    return null;
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
  });
})();

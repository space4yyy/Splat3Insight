import { availableLocales, currentLocale, localizeRecord, setLocale, t } from './i18n.js';

const app = document.querySelector('#app');

const variants = {
  a: 'variantClassic',
  b: 'variantPower',
  c: 'variantRibbon'
};

const escapeHtml = (value) => String(value ?? '')
  .replaceAll('&', '&amp;')
  .replaceAll('<', '&lt;')
  .replaceAll('>', '&gt;')
  .replaceAll('"', '&quot;')
  .replaceAll("'", '&#039;');

const image = (url, className, alt) => url
  ? `<img class="${className}" src="${escapeHtml(url)}" alt="${escapeHtml(alt)}" referrerpolicy="no-referrer" />`
  : `<div class="${className}" aria-label="${escapeHtml(alt)}"></div>`;

const displayNumber = (value, digits = 0) => value === null || value === undefined
  ? '—'
  : Number(value).toFixed(digits);

const displayTime = (value) => value
  ? new Intl.DateTimeFormat(currentLocale(), { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).format(new Date(value))
  : t('unknownTime');

const resultClass = (judgement) => judgement === 'WIN' ? 'result-win' : 'result-lose';

function metric(label, value) {
  return `<div><div class="small-label">${escapeHtml(label)}</div><div class="metric-value">${escapeHtml(value)}</div></div>`;
}

function statsFor(record) {
  const stats = record.stats || {};
  const kills = Number(stats.kills);
  const deaths = Number(stats.deaths);
  return {
    kills: displayNumber(stats.kills),
    deaths: displayNumber(stats.deaths),
    assists: displayNumber(stats.assists),
    kdRatio: Number.isFinite(kills) && Number.isFinite(deaths)
      ? deaths === 0 ? '∞' : displayNumber(kills / deaths, 1)
      : '—',
    power: displayNumber(record.power, 1),
    powerDelta: record.powerDelta === null || record.powerDelta === undefined
      ? ''
      : `${record.powerDelta > 0 ? '+' : ''}${displayNumber(record.powerDelta, 1)}`,
    powerDeltaClass: record.powerDelta > 0 ? 'power-delta-positive' : 'power-delta-negative'
  };
}

function variantA(record) {
  const stats = statsFor(record);
  return `
    <section class="overlay-card compact-a ${escapeHtml(record.modeClass)}">
      <div class="compact-a__stripe"></div>
      <div class="compact-a__context">
        <div class="compact-a__mode ${escapeHtml(record.modeClass)}">${escapeHtml(record.ruleLabel)}</div>
      </div>
      <div class="compact-a__power">
        <div class="compact-a__power-value">${stats.power}</div>
        ${stats.powerDelta ? `<div class="compact-a__power-delta ${stats.powerDeltaClass}">${escapeHtml(stats.powerDelta)}</div>` : ''}
      </div>
      <div class="compact-a__outcome">
        <div class="compact-a__result ${resultClass(record.judgement)}">${escapeHtml(record.judgement === 'WIN' ? t('win') : t('lose'))}</div>
        <div class="compact-a__kad"><b>${stats.kills}</b> K <b>${stats.assists}</b> A <b>${stats.deaths}</b> D</div>
      </div>
    </section>`;
}

function variantB(record, recentSummary) {
  const stats = statsFor(record);
  const summary = recentSummary || { win: 0, lose: 0, winRate: 0 };
  const bankaraModeLabel = record.mode === 'BANKARA' && record.bankaraMode
    ? t(record.bankaraMode === 'CHALLENGE' ? 'challenge' : 'open')
    : '';
  return `
    <section class="overlay-card compact-b ${escapeHtml(record.modeClass)}">
      <div class="compact-b__top">
        <div class="compact-b__mode ${escapeHtml(record.modeClass)}">
          <span>${escapeHtml(record.ruleLabel)}</span>
          ${bankaraModeLabel ? `<small class="compact-b__bankara-mode">${escapeHtml(bankaraModeLabel)}</small>` : ''}
        </div>
        <div class="compact-b__result-summary" aria-label="Win ${summary.win}, Lose ${summary.lose}, ${displayNumber(summary.winRate, 0)}%">
          <span class="compact-b__summary-win">Win ${escapeHtml(summary.win)}</span>
          <span class="compact-b__summary-lose">Lose ${escapeHtml(summary.lose)}</span>
          <span class="compact-b__summary-rate">${displayNumber(summary.winRate, 0)}%</span>
        </div>
      </div>
      <div class="compact-b__bottom">
        <div class="compact-b__power">
          <span class="compact-b__power-value">${stats.power}</span>
          ${stats.powerDelta ? `<span class="compact-b__power-delta ${stats.powerDeltaClass}">${escapeHtml(stats.powerDelta)}</span>` : ''}
        </div>
        <div class="compact-b__stats">
          <div class="compact-b__kad"><span>${stats.kills}<small>(${stats.assists})k</small></span><span>${stats.deaths}<small>d</small></span><span>${stats.kdRatio}<small>k/d</small></span></div>
        </div>
      </div>
    </section>`;
}

function variantC(record) {
  const stats = statsFor(record);
  return `
    <section class="overlay-card compact-c ${escapeHtml(record.modeClass)}">
      <div class="compact-c__mode ${escapeHtml(record.modeClass)}">${escapeHtml(record.ruleLabel)}</div>
      <div class="compact-c__power-block">
        <span class="compact-c__power">${stats.power}</span>
        ${stats.powerDelta ? `<span class="compact-c__power-delta ${stats.powerDeltaClass}">${escapeHtml(stats.powerDelta)}</span>` : ''}
      </div>
      <div class="compact-c__spacer"></div>
      <div class="compact-c__result ${resultClass(record.judgement)}">${escapeHtml(record.judgement === 'WIN' ? t('win') : t('lose'))}</div>
      <div class="compact-c__kad">${stats.kills}K&nbsp; ${stats.assists}A&nbsp; ${stats.deaths}D</div>
    </section>`;
}

function switcher(current) {
  const keys = Object.keys(variants);
  const index = keys.indexOf(current);
  const move = (direction) => {
    const next = keys[(index + direction + keys.length) % keys.length];
    const params = new URLSearchParams(location.search);
    params.set('variant', next);
    history.replaceState(null, '', `${location.pathname}?${params}`);
    render(window.__previewData, next);
  };

  return `
    <nav class="switcher" aria-label="Prototype variants">
      <button type="button" data-direction="-1" aria-label="${escapeHtml(t('previousVariant'))}">←</button>
      <div class="variant-label">${escapeHtml(t(variants[current]))}<small>${escapeHtml(t('compareHint'))}</small></div>
      <button type="button" data-direction="1" aria-label="${escapeHtml(t('nextVariant'))}">→</button>
    </nav>`;
}

function render(data, requestedVariant = 'a') {
  const variant = variants[requestedVariant] ? requestedVariant : 'a';
  const rawRecords = document.body.classList.contains('obs-mode')
    ? [data.selected]
    : [data.samples?.xMatch, data.samples?.bankara];
  const records = rawRecords.filter(Boolean).map(localizeRecord);
  const record = records[0] || null;
  if (!records.length) {
    app.innerHTML = `<div class="error">${escapeHtml(t('errorNoRecord'))}</div>`;
    return;
  }

  const cards = records.map((sample) => {
    const card = variant === 'b'
      ? variantB(sample, data.recentSummary)
      : variant === 'c' ? variantC(sample) : variantA(sample);
    return `<div class="comparison-item">${card}</div>`;
  }).join('');
  app.innerHTML = `
    <header class="prototype-header">
      <div>
        <span class="eyebrow">${escapeHtml(t('eyebrow'))}</span>
        <h1>${escapeHtml(t('title'))}</h1>
        <p>${escapeHtml(t('description'))}</p>
      </div>
      <div class="header-tools">
        <label class="language-control">
          <span>${escapeHtml(t('language'))}</span>
          <select id="locale-select" aria-label="${escapeHtml(t('language'))}">
            ${availableLocales().map(({ id, label }) => `<option value="${escapeHtml(id)}" ${id === currentLocale() ? 'selected' : ''}>${escapeHtml(label)}</option>`).join('')}
          </select>
        </label>
        <div class="source-chip">${escapeHtml(data.source.totalFiles)} ${escapeHtml(t('exported'))}</div>
      </div>
    </header>
    <section class="preview-frame"><div class="comparison-grid">${cards}</div></section>
    <div class="data-notes">
      <span>${escapeHtml(t('source'))}: ${escapeHtml(record.sourceFile)}</span>
      <span>${escapeHtml(t('dataPath'))}: vsHistoryDetail</span>
      <span>${escapeHtml(t('playerPath'))}: myTeam.players[isMyself]</span>
      <span>${escapeHtml(t('powerPath'))}: ${escapeHtml(record.powerPath || '—')}</span>
    </div>
    ${switcher(variant)}`;

  app.querySelector('#locale-select')?.addEventListener('change', async (event) => {
    await setLocale(event.target.value);
    localStorage.setItem('splat3-widget-locale', currentLocale());
    render(window.__previewData, variant);
  });

  app.querySelectorAll('.switcher button').forEach((button) => {
    button.addEventListener('click', () => moveVariant(Number(button.dataset.direction)));
  });
  app.querySelectorAll('img').forEach((img) => {
    img.addEventListener('error', () => img.classList.add('image-unavailable'), { once: true });
  });
}

function moveVariant(direction) {
  const current = new URLSearchParams(location.search).get('variant') || 'a';
  const keys = Object.keys(variants);
  const index = keys.indexOf(current);
  const next = keys[(index + direction + keys.length) % keys.length];
  const params = new URLSearchParams(location.search);
  params.set('variant', next);
  history.replaceState(null, '', `${location.pathname}?${params}`);
  render(window.__previewData, next);
}

document.addEventListener('keydown', (event) => {
  if (['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement?.tagName)) return;
  if (event.key === 'ArrowLeft') moveVariant(-1);
  if (event.key === 'ArrowRight') moveVariant(1);
});

if (new URLSearchParams(location.search).get('obs') === '1') document.body.classList.add('obs-mode');

const requestedLocale = new URLSearchParams(location.search).get('lang')
  || localStorage.getItem('splat3-widget-locale')
  || 'zh-CN';

fetch('/api/preview').then((response) => response.json()).then((data) => {
  window.__previewData = data;
  render(data, new URLSearchParams(location.search).get('variant') || 'a');
  return setLocale(requestedLocale).then(() => {
    render(data, new URLSearchParams(location.search).get('variant') || 'a');
  });
}).catch((error) => {
  app.innerHTML = `<div class="error">${escapeHtml(t('errorData'))}：${escapeHtml(error.message)}</div>`;
});

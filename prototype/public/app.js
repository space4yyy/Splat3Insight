import { availableLocales, currentLocale, localizeRecord, setLocale, t } from './i18n.js';

const app = document.querySelector('#app');

const escapeHtml = (value) => String(value ?? '')
  .replaceAll('&', '&amp;')
  .replaceAll('<', '&lt;')
  .replaceAll('>', '&gt;')
  .replaceAll('"', '&quot;')
  .replaceAll("'", '&#039;');

const displayNumber = (value, digits = 0) => value === null || value === undefined
  ? '—'
  : Number(value).toFixed(digits);

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

function render(data) {
  const rawRecords = document.body.classList.contains('obs-mode')
    ? [data.selected]
    : [data.samples?.xMatch, data.samples?.bankara];
  const records = rawRecords.filter(Boolean).map(localizeRecord);
  const record = records[0] || null;
  if (!records.length) {
    app.innerHTML = `<div class="error">${escapeHtml(t('errorNoRecord'))}</div>`;
    return;
  }

  const cards = records
    .map((sample) => `<div class="comparison-item">${variantB(sample, data.recentSummary)}</div>`)
    .join('');
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
  `;

  app.querySelector('#locale-select')?.addEventListener('change', async (event) => {
    await setLocale(event.target.value);
    localStorage.setItem('splat3-widget-locale', currentLocale());
    render(window.__previewData);
  });

  app.querySelectorAll('img').forEach((img) => {
    img.addEventListener('error', () => img.classList.add('image-unavailable'), { once: true });
  });
}

if (new URLSearchParams(location.search).get('obs') === '1') document.body.classList.add('obs-mode');

const requestedLocale = new URLSearchParams(location.search).get('lang')
  || localStorage.getItem('splat3-widget-locale')
  || 'zh-CN';

fetch('/api/preview').then((response) => response.json()).then((data) => {
  window.__previewData = data;
  render(data);
  return setLocale(requestedLocale).then(() => {
    render(data);
  });
}).catch((error) => {
  app.innerHTML = `<div class="error">${escapeHtml(t('errorData'))}：${escapeHtml(error.message)}</div>`;
});

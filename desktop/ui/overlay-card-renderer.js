(function () {
  'use strict';

  const ruleLabels = Object.freeze({
    '真格魚虎對戰': '真格鱼虎',
    '真格區域對戰': '真格区域',
    '真格塔樓對戰': '真格塔楼',
    '真格蛤蜊對戰': '真格蛤蜊',
    '真格魚虎': '真格鱼虎',
    '真格區域': '真格区域',
    '真格塔樓': '真格塔楼',
    Rainmaker: '真格鱼虎',
    'Splat Zones': '真格区域',
    'Tower Control': '真格塔楼',
    'Clam Blitz': '真格蛤蜊',
    'Turf War': '占地对战'
  });

  const escapeHtml = (value) => String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
  const isNumber = (value) => value !== null && value !== undefined && Number.isFinite(Number(value));
  const displayNumber = (value, digits = 0) => isNumber(value) ? Number(value).toFixed(digits) : '—';
  const placeholderClass = (value) => isNumber(value) ? '' : 'is-placeholder';
  const localizeRule = (value) => {
    const label = String(value ?? '').trim();
    return ruleLabels[label] || label || '—';
  };

  function render(state) {
    const battle = state?.selected;
    if (!battle) return '';

    const hasKills = isNumber(battle.kills);
    const hasDeaths = isNumber(battle.deaths);
    const kd = hasKills && hasDeaths
      ? Number(battle.deaths) === 0 ? '∞' : displayNumber(Number(battle.kills) / Number(battle.deaths), 1)
      : '—';
    const delta = isNumber(battle.power_delta)
      ? `${battle.power_delta >= 0 ? '+' : ''}${displayNumber(battle.power_delta, 1)}`
      : '—';
    const deltaClass = battle.power_delta > 0 ? 'positive' : battle.power_delta < 0 ? 'negative' : '';
    const mode = battle.mode === 'X_MATCH' ? 'mode-x-match' : 'mode-bankara';
    const bankaraMode = battle.bankara_mode === 'CHALLENGE' ? '挑战'
      : battle.bankara_mode === 'OPEN' ? '开放' : '';
    const summary = state.recent_summary || { win: 0, lose: 0, win_rate: 0 };

    return `
      <section class="overlay-card">
        <div class="overlay-card__top">
          <div class="overlay-card__mode ${mode}">
            <span>${escapeHtml(localizeRule(battle.rule))}</span>
            ${bankaraMode ? `<small class="overlay-card__bankara-mode">${bankaraMode}</small>` : ''}
          </div>
          <div class="overlay-card__summary">
            <span class="overlay-card__summary-win">Win ${displayNumber(summary.win)}</span>
            <span class="overlay-card__summary-lose">Lose ${displayNumber(summary.lose)}</span>
            <span>${displayNumber(summary.win_rate)}%</span>
          </div>
        </div>
        <div class="overlay-card__bottom">
          <div class="overlay-card__power">
            <span class="overlay-card__power-value ${placeholderClass(battle.power)}">${displayNumber(battle.power, 1)}</span>
            <span class="overlay-card__power-delta ${deltaClass} ${placeholderClass(battle.power_delta)}">${delta}</span>
          </div>
          <div class="overlay-card__stats">
            <div class="overlay-card__kad">
              <span class="${placeholderClass(battle.kills)}">${displayNumber(battle.kills)}<small>(${displayNumber(battle.assists)})k</small></span>
              <span class="${placeholderClass(battle.deaths)}">${displayNumber(battle.deaths)}<small>d</small></span>
              <span class="${!hasKills || !hasDeaths ? 'is-placeholder' : ''}">${kd}<small>k/d</small></span>
            </div>
          </div>
        </div>
      </section>`;
  }

  window.Splat3OverlayCard = Object.freeze({ render });
})();

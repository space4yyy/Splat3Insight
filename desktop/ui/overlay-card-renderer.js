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

  const mountedCards = new WeakMap();
  const isNumber = (value) => value !== null && value !== undefined && Number.isFinite(Number(value));
  const displayNumber = (value, digits = 0) => isNumber(value) ? Number(value).toFixed(digits) : '—';
  const localizeRule = (value) => {
    const label = String(value ?? '').trim();
    return ruleLabels[label] || label || '—';
  };

  function createCard(root) {
    root.innerHTML = `
      <section class="overlay-card">
        <div class="overlay-card__top">
          <div class="overlay-card__mode">
            <span data-field="rule"></span>
            <small class="overlay-card__bankara-mode" data-field="bankara-mode" hidden></small>
          </div>
          <div class="overlay-card__summary">
            <span class="overlay-card__summary-win" data-field="wins"></span>
            <span class="overlay-card__summary-lose" data-field="losses"></span>
            <span data-field="win-rate"></span>
          </div>
        </div>
        <div class="overlay-card__bottom">
          <div class="overlay-card__power">
            <span class="overlay-card__power-value" data-field="power"></span>
            <span class="overlay-card__power-delta" data-field="delta"></span>
          </div>
          <div class="overlay-card__stats">
            <div class="overlay-card__kad">
              <span data-field="kills">—<small data-field="assists"></small></span>
              <span data-field="deaths">—<small>d</small></span>
              <span data-field="kd">—<small>k/d</small></span>
            </div>
          </div>
        </div>
      </section>`;

    const card = root.querySelector('.overlay-card');
    const field = (name) => card.querySelector(`[data-field="${name}"]`);
    const refs = {
      card,
      mode: card.querySelector('.overlay-card__mode'),
      rule: field('rule'),
      bankaraMode: field('bankara-mode'),
      wins: field('wins'),
      losses: field('losses'),
      winRate: field('win-rate'),
      power: field('power'),
      delta: field('delta'),
      kills: field('kills'),
      assists: field('assists'),
      deaths: field('deaths'),
      kd: field('kd'),
      killValue: field('kills').firstChild,
      deathValue: field('deaths').firstChild,
      kdValue: field('kd').firstChild
    };
    mountedCards.set(root, refs);
    return refs;
  }

  function setPlaceholder(element, value) {
    element.classList.toggle('is-placeholder', !isNumber(value));
  }

  function update(root, state) {
    const battle = state?.selected;
    let refs = mountedCards.get(root);

    if (!battle) {
      if (refs) refs.card.hidden = true;
      return false;
    }

    if (!refs) refs = createCard(root);
    refs.card.hidden = false;

    const summary = state.recent_summary || { win: 0, lose: 0, win_rate: 0 };
    const bankaraMode = battle.bankara_mode === 'CHALLENGE' ? '挑战'
      : battle.bankara_mode === 'OPEN' ? '开放' : '';
    const hasKills = isNumber(battle.kills);
    const hasDeaths = isNumber(battle.deaths);
    const kd = hasKills && hasDeaths
      ? Number(battle.deaths) === 0 ? '∞' : displayNumber(Number(battle.kills) / Number(battle.deaths), 1)
      : '—';
    const delta = isNumber(battle.power_delta)
      ? `${battle.power_delta >= 0 ? '+' : ''}${displayNumber(battle.power_delta, 1)}`
      : '—';

    refs.rule.textContent = localizeRule(battle.rule);
    refs.bankaraMode.textContent = bankaraMode;
    refs.bankaraMode.hidden = !bankaraMode;
    refs.mode.classList.toggle('mode-x-match', battle.mode === 'X_MATCH');
    refs.mode.classList.toggle('mode-bankara', battle.mode === 'BANKARA');
    refs.mode.classList.toggle('mode-event-match', battle.mode === 'EVENT_MATCH');

    refs.wins.textContent = `Win ${displayNumber(summary.win)}`;
    refs.losses.textContent = `Lose ${displayNumber(summary.lose)}`;
    refs.winRate.textContent = `${displayNumber(summary.win_rate)}%`;

    refs.power.textContent = displayNumber(battle.power, 1);
    setPlaceholder(refs.power, battle.power);
    refs.delta.textContent = delta;
    refs.delta.classList.toggle('positive', battle.power_delta > 0);
    refs.delta.classList.toggle('negative', battle.power_delta < 0);
    setPlaceholder(refs.delta, battle.power_delta);

    refs.killValue.nodeValue = displayNumber(battle.kills);
    refs.assists.textContent = `(${displayNumber(battle.assists)})k`;
    setPlaceholder(refs.kills, battle.kills);
    refs.deathValue.nodeValue = displayNumber(battle.deaths);
    setPlaceholder(refs.deaths, battle.deaths);
    refs.kdValue.nodeValue = kd;
    refs.kd.classList.toggle('is-placeholder', !hasKills || !hasDeaths);
    return true;
  }

  window.Splat3OverlayCard = Object.freeze({ update });
})();

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
  const mountedSalmonCards = new WeakMap();
  const mountedCardTypes = new WeakMap();
  const isNumber = (value) => value !== null && value !== undefined && Number.isFinite(Number(value));
  const displayNumber = (value, digits = 0) => isNumber(value) ? Number(value).toFixed(digits) : '—';
  const localizeRule = (value) => {
    const label = String(value ?? '').trim();
    return ruleLabels[label] || label || '—';
  };

  const salmonStageLabels = Object.freeze({
    'Bonerattle Arena': '鲑鱼心脏斗技场',
    'Gone Fission Hydroplant': '麦年海洋发电所',
    "Jammin' Salmon Junction": '生筋子系统交流道遗址',
    "Marooner's Bay": '漂浮落难船',
    'Salmonid Smokeyard': '时不知鲑烟熏工房',
    'Sockeye Station': '新卷堡',
    'Spawning Grounds': '鲑坝'
  });

  const salmonGradeLabels = Object.freeze({
    'Eggsecutive VP': '传说',
    Profreshional: '达人'
  });

  function prepareCardType(root, type) {
    const currentType = mountedCardTypes.get(root);
    if (currentType && currentType !== type) {
      mountedCards.delete(root);
      mountedSalmonCards.delete(root);
      root.replaceChildren();
    }
    mountedCardTypes.set(root, type);
  }

  function autoCardType(state) {
    if (!state?.salmon_run) return 'overlay';
    if (!state.selected) return 'salmon-run';
    const salmonRunTime = Date.parse(state.salmon_run.played_time ?? '');
    const battleTime = Date.parse(state.selected.played_time ?? '');
    return Number.isFinite(salmonRunTime)
      && (!Number.isFinite(battleTime) || salmonRunTime > battleTime)
      ? 'salmon-run'
      : 'overlay';
  }

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
    prepareCardType(root, 'overlay');
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
    const isPrivateBattle = battle.mode === 'PRIVATE_BATTLE';
    refs.mode.classList.toggle('mode-x-match', battle.mode === 'X_MATCH');
    refs.mode.classList.toggle('mode-bankara', battle.mode === 'BANKARA');
    refs.mode.classList.toggle('mode-event-match', battle.mode === 'EVENT_MATCH');
    refs.mode.classList.toggle('mode-private-battle', isPrivateBattle);

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

  function createSalmonRunCard(root) {
    root.innerHTML = `
      <section class="salmon-run-card">
        <div class="salmon-run-card__top">
          <span class="salmon-run-card__stage" data-salmon-field="stage"></span>
          <div class="salmon-run-card__metadata">
            <div class="salmon-run-card__details-grade"><span data-salmon-field="grade"></span><b data-salmon-field="grade-point"></b></div>
            <div class="salmon-run-card__details-danger">危险度 <b data-salmon-field="danger-rate"></b></div>
          </div>
        </div>
        <div class="salmon-run-card__bottom">
          <div class="salmon-run-card__stats">
            <span class="salmon-run-card__stats-item">巨大鲑鱼 <b data-salmon-field="defeat-enemy-count"></b></span>
            <span class="salmon-run-card__stats-item"><img class="salmon-run-card__stats-icon" data-salmon-stat-icon="golden-egg" alt="搬蛋"><b data-salmon-field="golden-egg-count"></b></span>
            <span class="salmon-run-card__stats-item"><img class="salmon-run-card__stats-icon" data-salmon-stat-icon="power-egg" alt="红蛋"><b data-salmon-field="power-egg-count"></b></span>
            <span class="salmon-run-card__stats-item"><img class="salmon-run-card__stats-icon" data-salmon-stat-icon="rescue" alt="救援"><b data-salmon-field="rescue-count"></b></span>
            <span class="salmon-run-card__stats-item"><img class="salmon-run-card__stats-icon" data-salmon-stat-icon="rescued" alt="被救"><b data-salmon-field="rescued-count"></b></span>
          </div>
          <div class="salmon-run-card__details">
            <div class="salmon-run-card__scales" aria-label="鳞片总计">
              <span class="salmon-run-card__scales-label">总计:</span>
              <span class="salmon-run-card__scale"><img class="salmon-run-card__scale-icon" data-salmon-scale="gold" alt="金鳞片"><b class="salmon-run-card__number" data-salmon-field="gold-scale"></b></span>
              <span class="salmon-run-card__scale"><img class="salmon-run-card__scale-icon" data-salmon-scale="silver" alt="银鳞片"><b class="salmon-run-card__number" data-salmon-field="silver-scale"></b></span>
              <span class="salmon-run-card__scale"><img class="salmon-run-card__scale-icon" data-salmon-scale="bronze" alt="铜鳞片"><b class="salmon-run-card__number" data-salmon-field="bronze-scale"></b></span>
            </div>
            <div class="salmon-run-card__boss" data-salmon-boss hidden>
              <span class="salmon-run-card__boss-gauge" data-salmon-boss-gauge role="img">
                <span class="salmon-run-card__boss-gauge-fill"></span>
              </span>
              <b data-salmon-field="boss-encounter-count"></b>
              <b class="salmon-run-card__boss-undefeated" data-salmon-field="boss-undefeated-count"></b>
            </div>
            <div class="salmon-run-card__summary">
              <span>场次<b data-salmon-field="total-count"></b><span class="salmon-run-card__summary-failure">(<b data-salmon-field="failure-count"></b>)</span></span>
            </div>
          </div>
        </div>
      </section>`;

    const card = root.querySelector('.salmon-run-card');
    const fields = Object.fromEntries(
      Array.from(card.querySelectorAll('[data-salmon-field]'))
        .map((element) => [element.dataset.salmonField, element])
    );
    for (const image of card.querySelectorAll('[data-salmon-scale]')) {
      const kind = image.dataset.salmonScale;
      image.src = new URL(`./images/scale-${kind}.png`, document.baseURI).href;
    }
    for (const image of card.querySelectorAll('[data-salmon-stat-icon]')) {
      const kind = image.dataset.salmonStatIcon;
      const filename = kind === 'golden-egg' ? 'gold-ikura.svg' : 'ikura.svg';
      image.src = new URL(`./images/${filename}`, document.baseURI).href;
    }
    const refs = { card, fields };
    mountedSalmonCards.set(root, refs);
    return refs;
  }

  function updateSalmonRun(root, state) {
    prepareCardType(root, 'salmon-run');
    const data = state?.salmon_run;
    let refs = mountedSalmonCards.get(root);
    if (!data) {
      if (refs) refs.card.hidden = true;
      return false;
    }
    if (!refs) refs = createSalmonRunCard(root);
    refs.card.hidden = false;

    const species = String(data.species ?? '').trim().toUpperCase();
    const speciesAsset = species === 'INKLING' ? 'squid' : 'octo';
    for (const image of refs.card.querySelectorAll('[data-salmon-stat-icon="rescue"], [data-salmon-stat-icon="rescued"]')) {
      const kind = image.dataset.salmonStatIcon;
      const imageSpecies = species === 'INKLING' ? 'INKLING' : 'OCTOLING';
      image.dataset.salmonSpecies = imageSpecies;
      const assetPrefix = kind === 'rescue' ? 'helpen' : 'helped';
      const imageUrl = new URL(`./images/${assetPrefix}-${speciesAsset}.svg`, document.baseURI).href;
      if (image.src !== imageUrl) image.src = imageUrl;
    }

    const stage = String(data.stage ?? '').trim();
    refs.fields.stage.textContent = salmonStageLabels[stage] || stage || '—';
    refs.fields['gold-scale'].textContent = `x${displayNumber(data.gold_scale)}`;
    refs.fields['silver-scale'].textContent = `x${displayNumber(data.silver_scale)}`;
    refs.fields['bronze-scale'].textContent = `x${displayNumber(data.bronze_scale)}`;
    refs.fields['defeat-enemy-count'].textContent = `x${displayNumber(data.defeat_enemy_count)}`;
    refs.fields['golden-egg-count'].textContent = `x${displayNumber(data.golden_egg_count)}`;
    refs.fields['power-egg-count'].textContent = `x${displayNumber(data.power_egg_count)}`;
    refs.fields['rescued-count'].textContent = `x${displayNumber(data.rescued_count)}`;
    refs.fields['rescue-count'].textContent = `x${displayNumber(data.rescue_count)}`;
    refs.fields['total-count'].textContent = isNumber(data.clear_count) && isNumber(data.failure_count)
      ? `x${displayNumber(Number(data.clear_count) + Number(data.failure_count))}`
      : '—';
    refs.fields['failure-count'].textContent = displayNumber(data.failure_count);
    const bossIcons = {
      23: { mask: 'boss1.png', color: 'boss1_color.png' },
      24: { mask: 'boss2.png', color: 'boss2_color.png' },
      25: { mask: 'boss3.png', color: 'boss3_color.png' },
      30: { mask: 'boss_all.png', color: 'boss_all_color.png' },
    };
    const bossAsset = bossIcons[Number(data.boss_enemy_id)];
    const bossCountAvailable = isNumber(data.boss_encounter_count) && isNumber(data.boss_undefeated_count);
    const bossWidget = refs.card.querySelector('[data-salmon-boss]');
    if (bossAsset && bossCountAvailable) {
      const gauge = refs.card.querySelector('[data-salmon-boss-gauge]');
      const progress = isNumber(data.boss_progress) ? Math.max(0, Math.min(5, Number(data.boss_progress))) : 0;
      const waveYScale = [0, 0.2, 0.4, 0.6, 0.8, 2][progress];
      gauge.style.setProperty('--boss-image', `url("./images/${bossAsset.mask}")`);
      gauge.style.setProperty('--boss-color-image', `url("./images/${bossAsset.color}")`);
      gauge.style.setProperty('--boss-wave-y-scale', `${waveYScale * 100}%`);
      gauge.dataset.bossProgress = String(progress);
      gauge.setAttribute('aria-label', `头目出现进度 ${progress}/5`);
      refs.fields['boss-encounter-count'].textContent = `x${displayNumber(data.boss_encounter_count)}`;
      refs.fields['boss-undefeated-count'].textContent = `(${displayNumber(data.boss_undefeated_count)})`;
      bossWidget.hidden = false;
    } else {
      bossWidget.hidden = true;
    }
    const rawGrade = String(data.grade ?? '').trim();
    refs.fields.grade.textContent = salmonGradeLabels[rawGrade] || rawGrade;
    refs.fields['grade-point'].textContent = isNumber(data.grade_point) ? displayNumber(data.grade_point) : '';
    if (isNumber(data.danger_rate)) {
      const dangerRate = Number(data.danger_rate);
      const dangerPercent = dangerRate < 10 ? dangerRate * 100 : dangerRate;
      refs.fields['danger-rate'].textContent = `${Number(dangerPercent.toFixed(1))}%`;
    } else {
      refs.fields['danger-rate'].textContent = '—';
    }
    return true;
  }

  function updateAuto(root, state) {
    const type = autoCardType(state);
    if (type === 'salmon-run') {
      updateSalmonRun(root, state);
    } else {
      update(root, state);
    }
    return type;
  }

  window.Splat3OverlayCard = Object.freeze({ update, updateAuto, updateSalmonRun });
})();

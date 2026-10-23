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

  // 名称查官方游戏文本（game-names.js），查不到时原样显示。
  const officialName = (kind, text) => window.Splat3GameNames?.lookup(kind, text) ?? null;

  const mountedCards = new WeakMap();
  const mountedSalmonCards = new WeakMap();
  const mountedClockCards = new WeakMap();
  const mountedDefeatCards = new WeakMap();
  const mountedCardTypes = new WeakMap();
  const isNumber = (value) => value !== null && value !== undefined && Number.isFinite(Number(value));
  const displayNumber = (value, digits = 0) => isNumber(value) ? Number(value).toFixed(digits) : '—';

  // 数字变化时滚动到新值并短暂发亮；第一次显示、没有变化或不是数字时直接替换。
  // target 可以是元素，也可以是元素里的文字节点（击倒、死亡、K/D 的数字后面还跟着单位）。
  // 带千位分隔符的数字（击倒数量的 4,812）滚动时也保留分隔符。
  const NUMBER_PATTERN = /^([^\d-]*)(-?\d{1,3}(?:,\d{3})+|-?\d+(?:\.\d+)?)(.*)$/;
  const groupDigits = (text) => String(text).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const NUMBER_ROLL_MS = 800;
  const NUMBER_GLOW_MS = 1200;
  const rollingNumbers = new WeakMap();
  const glowTimers = new WeakMap();
  const reducedMotion = () => Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
  function showNumber(target, text) {
    const isTextNode = target.nodeType === 3;
    const element = isTextNode ? target.parentElement : target;
    const write = (value) => {
      if (isTextNode) target.nodeValue = value;
      else target.textContent = value;
    };
    const previousRoll = rollingNumbers.get(target);
    const current = previousRoll ? previousRoll.text : (isTextNode ? target.nodeValue : target.textContent);
    if (current === text) return;
    if (previousRoll) previousRoll.stop();
    const shownBefore = element.dataset.numberShown === 'true';
    element.dataset.numberShown = 'true';
    if (!shownBefore || !window.requestAnimationFrame || reducedMotion()) {
      write(text);
      return;
    }
    const from = NUMBER_PATTERN.exec(current);
    const to = NUMBER_PATTERN.exec(text);
    if (from && to && from[1] === to[1] && from[3] === to[3]) {
      const start = Number(from[2].replace(/,/g, ''));
      const end = Number(to[2].replace(/,/g, ''));
      const digits = to[2].split('.')[1]?.length ?? 0;
      const grouped = from[2].includes(',') || to[2].includes(',');
      const format = (value) => (grouped ? groupDigits(value.toFixed(0)) : value.toFixed(digits));
      let began = null;
      const roll = { text, frame: 0, timer: 0 };
      roll.stop = () => {
        window.cancelAnimationFrame?.(roll.frame);
        clearTimeout(roll.timer);
        if (rollingNumbers.get(target) === roll) rollingNumbers.delete(target);
      };
      const finish = () => {
        roll.stop();
        write(text);
      };
      const step = (now) => {
        began ??= now;
        const progress = Math.min(1, Math.max(0, (now - began) / NUMBER_ROLL_MS));
        if (progress >= 1) {
          finish();
          return;
        }
        const eased = 1 - (1 - progress) ** 3;
        write(`${to[1]}${format(start + (end - start) * eased)}${to[3]}`);
        roll.frame = window.requestAnimationFrame(step);
      };
      rollingNumbers.set(target, roll);
      roll.frame = window.requestAnimationFrame(step);
      // 页面在后台时浏览器可能不再刷新画面，到点直接写上最终值。
      roll.timer = setTimeout(finish, NUMBER_ROLL_MS + 200);
    } else {
      write(text);
    }
    clearTimeout(glowTimers.get(element));
    element.classList.remove('is-number-changed');
    void element.offsetWidth;
    element.classList.add('is-number-changed');
    glowTimers.set(element, setTimeout(() => element.classList.remove('is-number-changed'), NUMBER_GLOW_MS));
  }

  // 规则优先用卡片自己的简称（真格鱼虎等），其余查官方文本。
  const localizeRule = (value) => {
    const label = String(value ?? '').trim();
    return ruleLabels[label] || officialName('rules', label) || label || '—';
  };
  const localizeCoopStage = (value) => {
    const stage = String(value ?? '').trim();
    return officialName('coopStages', stage) || stage || '—';
  };
  const localizeGrade = (value) => {
    const grade = String(value ?? '').trim();
    return officialName('grades', grade) || grade;
  };

  function prepareCardType(root, type) {
    const currentType = mountedCardTypes.get(root);
    if (currentType && currentType !== type) {
      mountedCards.delete(root);
      mountedSalmonCards.delete(root);
      mountedClockCards.delete(root);
      mountedDefeatCards.delete(root);
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
      powerArea: card.querySelector('.overlay-card__power'),
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

    showNumber(refs.wins, `Win ${displayNumber(summary.win)}`);
    showNumber(refs.losses, `Lose ${displayNumber(summary.lose)}`);
    showNumber(refs.winRate, `${displayNumber(summary.win_rate)}%`);

    // 蛮颓开放组队定分中：战力的位置写「定分中」，变化值的位置写打了几场定分。
    const placement = battle.power_placement;
    refs.powerArea.classList.toggle('is-placement', Boolean(placement));
    if (placement) {
      const progress = isNumber(placement.played) && isNumber(placement.required)
        ? `${placement.played}/${placement.required}`
        : '';
      showNumber(refs.power, '定分中');
      refs.power.classList.remove('is-placeholder');
      showNumber(refs.delta, progress);
      refs.delta.classList.remove('positive', 'negative', 'is-placeholder');
    } else {
      showNumber(refs.power, displayNumber(battle.power, 1));
      setPlaceholder(refs.power, battle.power);
      showNumber(refs.delta, delta);
      refs.delta.classList.toggle('positive', battle.power_delta > 0);
      refs.delta.classList.toggle('negative', battle.power_delta < 0);
      setPlaceholder(refs.delta, battle.power_delta);
    }

    showNumber(refs.killValue, displayNumber(battle.kills));
    showNumber(refs.assists, `(${displayNumber(battle.assists)})k`);
    setPlaceholder(refs.kills, battle.kills);
    showNumber(refs.deathValue, displayNumber(battle.deaths));
    setPlaceholder(refs.deaths, battle.deaths);
    showNumber(refs.kdValue, kd);
    refs.kd.classList.toggle('is-placeholder', !hasKills || !hasDeaths);
    return true;
  }

  // 鲑鱼跑卡片第三行：鳞片、头目和场次在本期工里的总计。OBS 卡片和数据面板共用。
  const SALMON_TOTALS_HTML = `
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
          </div>`;
  const salmonBossIcons = Object.freeze({
    23: { mask: 'boss1.png', color: 'boss1_color.png' },
    24: { mask: 'boss2.png', color: 'boss2_color.png' },
    25: { mask: 'boss3.png', color: 'boss3_color.png' },
    30: { mask: 'boss_all.png', color: 'boss_all_color.png' },
  });

  function salmonFields(scope) {
    return Object.fromEntries(
      Array.from(scope.querySelectorAll('[data-salmon-field]'))
        .map((element) => [element.dataset.salmonField, element])
    );
  }

  function setSalmonScaleImages(scope) {
    for (const image of scope.querySelectorAll('[data-salmon-scale]')) {
      const kind = image.dataset.salmonScale;
      image.src = new URL(`./images/scale-${kind}.png`, document.baseURI).href;
    }
  }

  function fillSalmonTotals(scope, fields, data) {
    showNumber(fields['gold-scale'], `x${displayNumber(data.gold_scale)}`);
    showNumber(fields['silver-scale'], `x${displayNumber(data.silver_scale)}`);
    showNumber(fields['bronze-scale'], `x${displayNumber(data.bronze_scale)}`);
    showNumber(fields['total-count'], isNumber(data.clear_count) && isNumber(data.failure_count)
      ? `x${displayNumber(Number(data.clear_count) + Number(data.failure_count))}`
      : '—');
    showNumber(fields['failure-count'], displayNumber(data.failure_count));
    const bossAsset = salmonBossIcons[Number(data.boss_enemy_id)];
    const bossCountAvailable = isNumber(data.boss_encounter_count) && isNumber(data.boss_undefeated_count);
    const bossWidget = scope.querySelector('[data-salmon-boss]');
    if (bossAsset && bossCountAvailable) {
      const gauge = scope.querySelector('[data-salmon-boss-gauge]');
      const progress = isNumber(data.boss_progress) ? Math.max(0, Math.min(5, Number(data.boss_progress))) : 0;
      const waveYScale = [0, 0.2, 0.4, 0.6, 0.8, 2][progress];
      gauge.style.setProperty('--boss-image', `url("./images/${bossAsset.mask}")`);
      gauge.style.setProperty('--boss-color-image', `url("./images/${bossAsset.color}")`);
      gauge.style.setProperty('--boss-wave-y-scale', `${waveYScale * 100}%`);
      gauge.dataset.bossProgress = String(progress);
      gauge.setAttribute('aria-label', `头目出现进度 ${progress}/5`);
      showNumber(fields['boss-encounter-count'], `x${displayNumber(data.boss_encounter_count)}`);
      showNumber(fields['boss-undefeated-count'], `(${displayNumber(data.boss_undefeated_count)})`);
      bossWidget.hidden = false;
    } else {
      bossWidget.hidden = true;
    }
  }

  // 只显示第三行的总计，供数据面板的“当前打工”使用。
  const mountedSalmonTotals = new WeakMap();
  function updateSalmonTotals(root, state) {
    const data = state?.salmon_run;
    let refs = mountedSalmonTotals.get(root);
    if (!data) {
      if (refs) refs.scope.hidden = true;
      return false;
    }
    if (!refs) {
      root.innerHTML = SALMON_TOTALS_HTML;
      const scope = root.querySelector('.salmon-run-card__details');
      setSalmonScaleImages(scope);
      refs = { scope, fields: salmonFields(scope) };
      mountedSalmonTotals.set(root, refs);
    }
    refs.scope.hidden = false;
    fillSalmonTotals(refs.scope, refs.fields, data);
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
${SALMON_TOTALS_HTML}
        </div>
      </section>`;

    const card = root.querySelector('.salmon-run-card');
    const fields = salmonFields(card);
    setSalmonScaleImages(card);
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
    refs.fields.stage.textContent = localizeCoopStage(stage);
    showNumber(refs.fields['defeat-enemy-count'], `x${displayNumber(data.defeat_enemy_count)}`);
    showNumber(refs.fields['golden-egg-count'], `x${displayNumber(data.golden_egg_count)}`);
    showNumber(refs.fields['power-egg-count'], `x${displayNumber(data.power_egg_count)}`);
    showNumber(refs.fields['rescued-count'], `x${displayNumber(data.rescued_count)}`);
    showNumber(refs.fields['rescue-count'], `x${displayNumber(data.rescue_count)}`);
    fillSalmonTotals(refs.card, refs.fields, data);
    refs.fields.grade.textContent = localizeGrade(data.grade);
    showNumber(refs.fields['grade-point'], isNumber(data.grade_point) ? displayNumber(data.grade_point) : '');
    if (isNumber(data.danger_rate)) {
      const dangerRate = Number(data.danger_rate);
      const dangerPercent = dangerRate < 10 ? dangerRate * 100 : dangerRate;
      showNumber(refs.fields['danger-rate'], `${Number(dangerPercent.toFixed(1))}%`);
    } else {
      showNumber(refs.fields['danger-rate'], '—');
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

  // 组合地址：在所选类型里取最近一条记录对应的卡片。
  // states 的键是地址参数（xmatch、bankaramatch、salmonrun），值是对应的卡片状态。
  function newestCardType(states) {
    let newest = null;
    for (const [type, state] of Object.entries(states || {})) {
      const playedTime = type === 'salmonrun' ? state?.salmon_run?.played_time : state?.selected?.played_time;
      const time = Date.parse(playedTime ?? '');
      if (Number.isFinite(time) && (!newest || time > newest.time)) newest = { type, time };
    }
    return newest?.type ?? null;
  }

  // 击倒数量卡片：「鲑鱼跑记录」里的累计击倒数量，一行图标加数字，巨大鲑鱼和头目鲑鱼之间用竖线隔开。
  // record 的 enemies（巨大鲑鱼）和 bosses（头目鲑鱼）按 SplatNet 的顺序，每项是 { id, name, image, count }。
  // image 交给 imageSrc 换成能显示的地址；头目鲑鱼没有图片时用卡片自带的彩色图。名称只放在提示里。
  function defeatCountItems(record, imageSrc = (url) => url) {
    const items = (group) => (Array.isArray(record?.[group]) ? record[group] : [])
      .filter((row) => isNumber(row?.count))
      .map((row) => {
        const id = isNumber(row.id) ? Number(row.id) : null;
        const name = String(row.name ?? '').trim();
        const bundled = group === 'bosses' && salmonBossIcons[id] ? `./images/${salmonBossIcons[id].color}` : null;
        return {
          key: `${group}:${id ?? name}`,
          name: officialName('bosses', name) || name,
          image: (row.image && imageSrc(row.image)) || bundled,
          fallback: bundled,
          count: Number(row.count),
        };
      });
    return { enemies: items('enemies'), bosses: items('bosses') };
  }

  function createDefeatCountCard(root, items) {
    const card = document.createElement('section');
    card.className = 'defeat-count-card';
    const counts = [];
    const addItem = (item) => {
      const cell = document.createElement('span');
      cell.className = 'defeat-count-card__item';
      cell.title = item.name;
      const icon = document.createElement('img');
      icon.className = 'defeat-count-card__icon';
      icon.alt = item.name;
      // 图片读不到时：头目鲑鱼换成自带的图，其他的留出空位，数字不挪位置。
      icon.addEventListener('error', () => {
        if (item.fallback && icon.getAttribute('src') !== item.fallback) icon.src = item.fallback;
        else icon.style.visibility = 'hidden';
      });
      if (item.image) icon.src = item.image;
      else icon.style.visibility = 'hidden';
      const count = document.createElement('b');
      count.className = 'defeat-count-card__count';
      cell.append(icon, count);
      card.append(cell);
      counts.push(count);
    };
    items.enemies.forEach(addItem);
    if (items.enemies.length && items.bosses.length) {
      const divider = document.createElement('span');
      divider.className = 'defeat-count-card__divider';
      card.append(divider);
    }
    items.bosses.forEach(addItem);
    root.replaceChildren(card);
    return { card, counts };
  }

  // 每次刷新只改数字（变化时滚动并发亮）；种类或图片变了才重建。
  function updateDefeatCounts(root, record, imageSrc) {
    prepareCardType(root, 'defeat-counts');
    const items = defeatCountItems(record, imageSrc);
    const list = [...items.enemies, ...items.bosses];
    let refs = mountedDefeatCards.get(root);
    if (!list.length) {
      if (refs) refs.card.hidden = true;
      return false;
    }
    const layout = list.map((item) => `${item.key} ${item.image}`).join('\n');
    if (!refs || refs.layout !== layout) {
      refs = { ...createDefeatCountCard(root, items), layout };
      mountedDefeatCards.set(root, refs);
    }
    refs.card.hidden = false;
    list.forEach((item, index) => {
      showNumber(refs.counts[index], groupDigits(item.count));
      refs.counts[index].classList.toggle('is-zero', item.count === 0);
    });
    return true;
  }

  // 时钟卡片：本机时区的当前时间，例如 16:31:09。
  // BlitzMain 的数字宽窄不一（1 比 0 窄很多），每位数字放进等宽格子，秒数跳动时整行不会左右晃。
  const CLOCK_FORMAT = '00:00:00';
  const twoDigits = (value) => String(value).padStart(2, '0');
  function clockText(now) {
    return `${twoDigits(now.getHours())}:${twoDigits(now.getMinutes())}:${twoDigits(now.getSeconds())}`;
  }

  function createClockCard(root) {
    const cells = Array.from(CLOCK_FORMAT, (char) => (char === '0' ? '<span class="clock-card__digit"></span>' : char)).join('');
    root.innerHTML = `
      <section class="clock-card">
        <time class="clock-card__time">${cells}</time>
      </section>`;
    const card = root.querySelector('.clock-card');
    const time = card.querySelector('.clock-card__time');
    const refs = { card, time, digits: Array.from(time.querySelectorAll('.clock-card__digit')) };
    mountedClockCards.set(root, refs);
    return refs;
  }

  function updateClock(root, now = new Date()) {
    prepareCardType(root, 'clock');
    const refs = mountedClockCards.get(root) ?? createClockCard(root);
    const text = clockText(now);
    const digits = text.replace(/\D/g, '');
    refs.digits.forEach((cell, index) => {
      if (cell.textContent !== digits[index]) cell.textContent = digits[index];
    });
    refs.time.dateTime = text;
    return true;
  }

  window.Splat3OverlayCard = Object.freeze({
    update,
    updateAuto,
    updateSalmonRun,
    updateSalmonTotals,
    newestCardType,
    updateDefeatCounts,
    defeatCountItems,
    updateClock,
    clockText,
    labels: Object.freeze({ rule: localizeRule, coopStage: localizeCoopStage, grade: localizeGrade }),
  });
})();

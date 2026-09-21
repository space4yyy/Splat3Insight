# Style prototype notes

Question being tested: what should a Splatoon 3 battle-result Overlay look like when driven by real `s3s` export data?

This is throwaway UI code. It intentionally has no live SplatNet login, no live Session, no cumulative stream statistics, and no persistence beyond the copied fixture data.

Variants are available at the same route:

- `?variant=a` — classic compact strip, closest to the original StreamingWidget reference
- `?variant=b` — power-focused two-row strip
- `?variant=c` — minimal one-line ribbon for maximum game visibility

The development preview renders the latest complete X Match and Anarchy Battle samples together. OBS mode keeps only the primary selected sample.

The parser currently reads `data.vsHistoryDetail`, finds `myTeam.players[]` with `isMyself: true`, and maps only the compact overlay fields: result, K/D/A, rule, mode color, the applicable power value, and the most recent non-zero change for that power track. X Match uses `xMatch.lastXPower`; Anarchy Open uses `bankaraMatch.bankaraPower.power` for team power; Anarchy Challenge uses `bankaraMatch.weaponPower` for weapon power. The mode name itself is intentionally omitted from the Overlay: X Match rules use `#0FDB9B`, Anarchy rules use `#F54910`, and the rule name carries the meaning. Map, weapon, paint, player name, and stage artwork remain available in the normalized record but are hidden from these variants.

Variant B also shows a shared recent-form summary: the latest 50 completed X Match and Anarchy Battle results are combined without separating the modes, then displayed as Win, Lose, and win rate in the top-right corner.

The prototype now includes `prototype/public/i18n.js` with Chinese, English, and Japanese locale switching. The official Leanny catalogs are loaded from:

The UI uses `prototype/public/fonts/DFP_GBZY7.ttf`, copied from `/Users/space4/Projects/s3-gear-generator/frontend/assets/fonts/DFP_GBZY7.ttf`.

- `https://leanny.github.io/splat3/data/language/CNzh.json`
- `https://leanny.github.io/splat3/data/language/USen.json`
- `https://leanny.github.io/splat3/data/language/JPja.json`

s3s names are reverse-matched against the US English catalog, then the same official key is read from the selected locale. Match modes, rules, stages, and main weapons are covered; unknown values fall back to the export text. The selected locale is available through `?lang=zh-CN`, `?lang=en-US`, or `?lang=ja-JP`, and is saved in local storage.

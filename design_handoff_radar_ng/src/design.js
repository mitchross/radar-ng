// Cumulus — Carrot-faithful visual system (no snark)
// Dark, saturated, dense. Condition-driven background gradients.
// Single brand accent: electric violet (#A78BFA-ish shifted toward indigo)

window.CUMULUS = window.CUMULUS || {};

window.CUMULUS.tokens = {
  // Accent (single, used for interactive moments + data)
  accent: '#8B7CFF',
  accentBright: '#A594FF',
  accentDim: '#5B4FD6',

  // Ink
  ink: '#FFFFFF',
  inkDim: 'rgba(255,255,255,0.72)',
  inkMuted: 'rgba(255,255,255,0.48)',
  inkFaint: 'rgba(255,255,255,0.28)',
  inkLine: 'rgba(255,255,255,0.10)',

  // Card surfaces — layered on top of condition bg
  card: 'rgba(255,255,255,0.06)',
  cardStrong: 'rgba(255,255,255,0.10)',
  cardLine: 'rgba(255,255,255,0.08)',

  // Data colors
  rain: '#4FB8FF',
  rainHeavy: '#1E7FFF',
  snow: '#C7E6FF',
  sun: '#FFC14D',
  temp: '#FF6E3A',
  cold: '#5BD4FF',
  hot: '#FF4D6D',
  alert: '#FF3B4A',
  ok: '#4ADE80',

  // Radar scale (Carrot-style, distinct from cool dashboard palette)
  dbz: {
     5: '#7ae5a8',   // light green
    15: '#3bc77a',   // green
    25: '#f5d042',   // yellow
    35: '#ff9f2e',   // orange
    45: '#ff4040',   // red
    55: '#d02058',   // crimson
    65: '#b24bff',   // violet
    75: '#ffffff',   // white (hail)
  },
};

// Condition-driven background gradients (applied to whole screen bg)
window.CUMULUS.conditionBG = {
  clearDay:   'radial-gradient(140% 90% at 70% -10%, #5B8FFF 0%, #2A3FA8 42%, #140E3D 100%)',
  clearNight: 'radial-gradient(140% 90% at 30% -10%, #2B2060 0%, #150B3D 50%, #050316 100%)',
  cloudy:     'linear-gradient(180deg, #3F4A6B 0%, #1E2540 55%, #0C1020 100%)',
  rain:       'linear-gradient(180deg, #2C5076 0%, #18304F 45%, #070D1C 100%)',
  storm:      'radial-gradient(120% 80% at 50% 0%, #5B2A7A 0%, #321551 45%, #0A0418 100%)',
  snow:       'linear-gradient(180deg, #4A5B78 0%, #26324E 50%, #0B1020 100%)',
  fog:        'linear-gradient(180deg, #4A5566 0%, #2A313F 55%, #0F131A 100%)',
};

// Typography — SF Pro Display for hero, SF Pro Rounded for UI, SF Mono for data
window.CUMULUS.fonts = {
  display: '"SF Pro Display", -apple-system, system-ui, sans-serif',
  ui:      '"SF Pro Rounded", "SF Pro Display", -apple-system, system-ui, sans-serif',
  mono:    '"SF Mono", "JetBrains Mono", ui-monospace, monospace',
};

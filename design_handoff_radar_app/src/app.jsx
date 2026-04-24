// Cumulus — App shell: tab nav + tweaks + data

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "condition": "rain",
  "timeOfDay": "day",
  "proMode": true,
  "accent": "violet",
  "screen": "home"
}/*EDITMODE-END*/;

const SETTINGS_DEFAULTS = {
  stackName: 'Home Lab · Cupertino',
  stackUrl: 'https://weather.local:8443',
  sources: {
    radar:     { url: 'https://weather.local:8081/mrms/{z}/{x}/{y}.png', status: 'healthy', auth: 'Bearer ••••7b2f' },
    satellite: { url: 'https://weather.local:8082/goes18/{z}/{x}/{y}.png', status: 'healthy' },
    forecast:  { url: 'https://weather.local:8083/hrrr/{layer}/{valid}.grib2', status: 'stale' },
    basemap:   { url: 'https://weather.local:8084/osm/{z}/{x}/{y}.pbf', status: 'healthy' },
    alerts:    { url: 'https://api.weather.gov/alerts/active', status: 'healthy' },
  },
  refresh: '1 min', cellular: true, vpn: true, preload: true, cdn: false,
  editing: null,
};

function App() {
  const [tweaks, setTweaks] = React.useState(TWEAK_DEFAULTS);
  const [editMode, setEditMode] = React.useState(false);
  const [screen, setScreen] = React.useState(tweaks.screen || 'home');
  const [settings, setSettings] = React.useState(SETTINGS_DEFAULTS);

  React.useEffect(() => {
    const onMsg = (e) => {
      if (e.data?.type === '__activate_edit_mode') setEditMode(true);
      if (e.data?.type === '__deactivate_edit_mode') setEditMode(false);
    };
    window.addEventListener('message', onMsg);
    window.parent.postMessage({ type: '__edit_mode_available' }, '*');
    return () => window.removeEventListener('message', onMsg);
  }, []);

  const updateTweak = (key, value) => {
    const next = { ...tweaks, [key]: value };
    setTweaks(next);
    window.parent.postMessage({ type: '__edit_mode_set_keys', edits: { [key]: value } }, '*');
  };

  // Apply accent
  React.useEffect(() => {
    const accents = {
      violet: { accent: '#8B7CFF', accentBright: '#A594FF', accentDim: '#5B4FD6' },
      teal:   { accent: '#2DD4BF', accentBright: '#5EEAD4', accentDim: '#0F766E' },
      amber:  { accent: '#F5A524', accentBright: '#FBBF5C', accentDim: '#B37418' },
      pink:   { accent: '#F472B6', accentBright: '#F9A8D4', accentDim: '#BE185D' },
    };
    Object.assign(window.CUMULUS.tokens, accents[tweaks.accent] || accents.violet);
  }, [tweaks.accent]);

  const data = buildData(tweaks);
  const bg = window.CUMULUS.conditionBG[
    tweaks.condition === 'clear' ? (tweaks.timeOfDay === 'night' ? 'clearNight' : 'clearDay')
    : tweaks.condition
  ] || window.CUMULUS.conditionBG.rain;

  return (
    <IOSDevice statusBarDark={false}>
      <div style={{
        width: '100%', height: '100%', position: 'relative',
        background: bg, color: '#fff', overflow: 'hidden',
      }}>
        {/* SVG filters (shared) */}
        <svg width="0" height="0" style={{ position: 'absolute' }}>
          <defs>
            <filter id="softBlur" x="-20%" y="-20%" width="140%" height="140%">
              <feGaussianBlur stdDeviation="4" />
            </filter>
          </defs>
        </svg>

        {/* Ambient flourishes for condition */}
        {tweaks.condition === 'rain' && <RainVeil />}
        {tweaks.condition === 'storm' && <StormFlash />}
        {tweaks.condition === 'snow' && <SnowVeil />}

        {screen === 'home' && <HomeScreen data={data} onOpenRadar={() => setScreen('radar')} onOpenNowcast={() => setScreen('nowcast')} onOpenSettings={() => setScreen('settings')} />}
        {screen === 'nowcast' && <NowcastScreen data={data.nowcastFull} onBack={() => setScreen('home')} />}
        {screen === 'radar' && <RadarScreen data={{ location: data.location }} onBack={() => setScreen('home')} />}
        {screen === 'settings' && <SettingsScreen onBack={() => setScreen('home')} settings={settings} onChange={setSettings} />}

        {/* Bottom tab bar (hidden on radar) */}
        {screen !== 'radar' && (
          <div style={{
            position: 'absolute', bottom: 18, left: 16, right: 16,
            height: 58, borderRadius: 22,
            background: 'rgba(10,10,20,0.55)', backdropFilter: 'blur(24px)',
            border: '1px solid rgba(255,255,255,0.12)',
            display: 'flex', alignItems: 'center', padding: '0 8px',
            zIndex: 10,
          }}>
            <TabBtn active={screen === 'home'} label="Home" onClick={() => setScreen('home')}>
              <svg width="18" height="18" viewBox="0 0 18 18" fill="none"><path d="M2 8 L9 2 L16 8 L16 16 L11 16 L11 11 L7 11 L7 16 L2 16 Z" stroke="#fff" strokeWidth="1.4" strokeLinejoin="round"/></svg>
            </TabBtn>
            <TabBtn active={screen === 'nowcast'} label="Nowcast" onClick={() => setScreen('nowcast')}>
              <svg width="18" height="18" viewBox="0 0 18 18" fill="none"><path d="M2 14 L6 10 L10 12 L16 4" stroke="#fff" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/><circle cx="16" cy="4" r="1.4" fill="#fff"/></svg>
            </TabBtn>
            <TabBtn active={screen === 'radar'} label="Radar" onClick={() => setScreen('radar')}>
              <svg width="18" height="18" viewBox="0 0 18 18" fill="none"><circle cx="9" cy="9" r="7" stroke="#fff" strokeWidth="1.3"/><circle cx="9" cy="9" r="4" stroke="#fff" strokeWidth="1.1" opacity="0.6"/><line x1="9" y1="9" x2="15" y2="4" stroke="#fff" strokeWidth="1.6" strokeLinecap="round"/></svg>
            </TabBtn>
            <TabBtn label="Alerts" count={data.alertCount}>
              <svg width="18" height="18" viewBox="0 0 18 18" fill="none"><path d="M9 2 L2 14 L16 14 Z" stroke="#fff" strokeWidth="1.4" strokeLinejoin="round"/><line x1="9" y1="7" x2="9" y2="10" stroke="#fff" strokeWidth="1.6" strokeLinecap="round"/><circle cx="9" cy="12" r="0.9" fill="#fff"/></svg>
            </TabBtn>
            <TabBtn active={screen === 'settings'} label="Settings" onClick={() => setScreen('settings')}>
              <svg width="18" height="18" viewBox="0 0 18 18" fill="none"><circle cx="9" cy="9" r="2" stroke="#fff" strokeWidth="1.4"/><path d="M9 1 V3 M9 15 V17 M1 9 H3 M15 9 H17 M3.3 3.3 L4.7 4.7 M13.3 13.3 L14.7 14.7 M3.3 14.7 L4.7 13.3 M13.3 4.7 L14.7 3.3" stroke="#fff" strokeWidth="1.4" strokeLinecap="round"/></svg>
            </TabBtn>
          </div>
        )}

        {editMode && <TweaksPanel tweaks={tweaks} onChange={updateTweak} />}
      </div>
    </IOSDevice>
  );
}

function TabBtn({ children, label, active, onClick, count }) {
  return (
    <div onClick={onClick} style={{
      flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2,
      cursor: 'pointer', padding: '6px 0',
      color: active ? '#fff' : 'rgba(255,255,255,0.55)',
      position: 'relative',
    }}>
      <div style={{ opacity: active ? 1 : 0.7 }}>{children}</div>
      <div style={{ fontSize: 9.5, fontWeight: 600, letterSpacing: '0.02em' }}>{label}</div>
      {count > 0 && (
        <div style={{ position: 'absolute', top: 2, right: '25%', width: 14, height: 14, borderRadius: 7, background: '#FF3B4A', fontSize: 9, fontWeight: 700, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{count}</div>
      )}
    </div>
  );
}

function TweaksPanel({ tweaks, onChange }) {
  const opts = (label, key, values) => (
    <div style={{ padding: '8px 10px 10px' }}>
      <div style={{ fontSize: 9, fontFamily: 'SF Mono', color: 'rgba(255,255,255,0.55)', letterSpacing: '0.12em', fontWeight: 600, marginBottom: 5 }}>{label}</div>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
        {values.map(v => (
          <div key={v} onClick={() => onChange(key, v)} style={{
            padding: '4px 9px', fontSize: 11, borderRadius: 8, cursor: 'pointer',
            background: tweaks[key] === v ? 'rgba(139,124,255,0.7)' : 'rgba(255,255,255,0.08)',
            border: `1px solid ${tweaks[key] === v ? 'rgba(139,124,255,1)' : 'rgba(255,255,255,0.1)'}`,
            fontWeight: 500, color: '#fff', textTransform: 'capitalize',
          }}>{v}</div>
        ))}
      </div>
    </div>
  );
  return (
    <div style={{
      position: 'absolute', right: 10, bottom: 90, width: 230,
      background: 'rgba(10,10,20,0.92)', backdropFilter: 'blur(20px)',
      border: '1px solid rgba(255,255,255,0.12)', borderRadius: 16,
      zIndex: 100, padding: '6px 2px 8px',
      boxShadow: '0 20px 60px rgba(0,0,0,0.5)',
    }}>
      <div style={{ padding: '6px 12px 2px', fontSize: 11, fontWeight: 700, letterSpacing: '0.06em', color: '#fff' }}>TWEAKS</div>
      {opts('CONDITION', 'condition', ['clear', 'cloudy', 'rain', 'storm', 'snow', 'fog'])}
      {opts('TIME', 'timeOfDay', ['day', 'night'])}
      {opts('ACCENT', 'accent', ['violet', 'teal', 'amber', 'pink'])}
      {opts('SCREEN', 'screen', ['home', 'nowcast', 'radar', 'settings'])}
    </div>
  );
}

// ─── Ambient veils
function RainVeil() {
  return (
    <svg width="100%" height="100%" style={{ position: 'absolute', inset: 0, pointerEvents: 'none', opacity: 0.35, zIndex: 1 }}>
      {Array.from({ length: 40 }).map((_, i) => {
        const x = (i * 37) % 400;
        const delay = (i * 0.12) % 2;
        return (
          <line key={i} x1={x} x2={x - 6} y1="-20" y2="8" stroke="#7DB8FF" strokeWidth="1" strokeLinecap="round">
            <animate attributeName="y1" values="-20;900" dur="1.8s" begin={`${delay}s`} repeatCount="indefinite" />
            <animate attributeName="y2" values="8;928" dur="1.8s" begin={`${delay}s`} repeatCount="indefinite" />
          </line>
        );
      })}
    </svg>
  );
}
function StormFlash() {
  return (
    <div style={{ position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 1, animation: 'stormFlash 7s infinite' }}>
      <style>{`@keyframes stormFlash { 0%, 90%, 100% { background: transparent; } 92%, 94% { background: rgba(255,255,255,0.12); } 93%, 95% { background: transparent; } }`}</style>
    </div>
  );
}
function SnowVeil() {
  return (
    <svg width="100%" height="100%" style={{ position: 'absolute', inset: 0, pointerEvents: 'none', opacity: 0.6, zIndex: 1 }}>
      {Array.from({ length: 30 }).map((_, i) => {
        const x = (i * 47) % 400;
        const delay = (i * 0.2) % 5;
        return (
          <circle key={i} cx={x} cy="-10" r={1 + (i % 3) * 0.5} fill="#fff">
            <animate attributeName="cy" values="-10;900" dur={`${4 + (i % 3)}s`} begin={`${delay}s`} repeatCount="indefinite" />
            <animate attributeName="cx" values={`${x};${x + 20};${x}`} dur="3s" repeatCount="indefinite" />
          </circle>
        );
      })}
    </svg>
  );
}

// ─── data
function buildData(tweaks) {
  const condition = tweaks.condition;
  const isRain = condition === 'rain' || condition === 'storm';
  const isClear = condition === 'clear';
  const isSnow = condition === 'snow';

  const hero = isRain ? (condition === 'storm' ? 'storm' : 'heavyRain')
             : isClear ? (tweaks.timeOfDay === 'night' ? 'moon' : 'sun')
             : isSnow ? 'snow'
             : condition === 'fog' ? 'fog'
             : 'cloudy';

  const temp = isSnow ? 28 : isClear ? (tweaks.timeOfDay === 'night' ? 58 : 74) : isRain ? 54 : 61;
  const feels = temp - (isRain ? 6 : isSnow ? 10 : 2);

  // 24h hourly
  const hourly = Array.from({ length: 24 }).map((_, i) => {
    const hr = (new Date().getHours() + i) % 24;
    const timeOfDay = (hr >= 6 && hr < 19) ? 'day' : 'night';
    let precip = 0;
    if (isRain) precip = Math.max(0, Math.round(80 * Math.exp(-Math.pow((i - 5) / 5, 2)) + (i > 2 ? 20 : 0)));
    else if (condition === 'cloudy') precip = Math.max(0, Math.round(30 * Math.sin(i / 3)));
    else if (isSnow) precip = i < 12 ? 60 : 30;
    const icon = i === 0 ? hero
               : isRain && i < 10 ? (i < 5 ? 'heavyRain' : 'rain')
               : isClear ? (timeOfDay === 'night' ? 'moon' : (i % 7 === 0 ? 'partlyCloudy' : 'sun'))
               : isSnow ? 'snow'
               : condition === 'fog' ? 'fog'
               : i % 3 === 0 ? 'partlyCloudy' : 'cloudy';
    return {
      time: hr === 0 ? '12a' : hr < 12 ? `${hr}a` : hr === 12 ? '12p' : `${hr - 12}p`,
      hr, timeOfDay, icon,
      temp: Math.round(temp + Math.sin(i / 4) * 6 - (i > 12 ? 3 : 0)),
      precip,
    };
  });

  // 7-day daily
  const daily = [
    { day: 'Today', icon: hero, hi: temp + 4, lo: temp - 12, precip: isRain ? 85 : isClear ? 0 : 20, now: temp },
    { day: 'Tue', icon: isRain ? 'rain' : 'partlyCloudy', hi: 72, lo: 54, precip: 45 },
    { day: 'Wed', icon: 'sun', hi: 78, lo: 56, precip: 0 },
    { day: 'Thu', icon: 'partlyCloudy', hi: 75, lo: 58, precip: 10 },
    { day: 'Fri', icon: 'heavyRain', hi: 68, lo: 55, precip: 90 },
    { day: 'Sat', icon: 'storm', hi: 65, lo: 52, precip: 95 },
    { day: 'Sun', icon: 'partlyCloudy', hi: 70, lo: 54, precip: 20 },
  ];
  const weekHi = Math.max(...daily.map(d => d.hi));
  const weekLo = Math.min(...daily.map(d => d.lo));

  // Nowcast minutes
  const minutes = Array.from({ length: 60 }).map((_, i) => {
    const baseI = isRain
      ? Math.max(0, Math.exp(-Math.pow((i - 25) / 14, 2)) * 0.85 + (i > 10 && i < 45 ? 0.1 : 0))
      : condition === 'cloudy' ? (i > 40 ? (i - 40) / 60 : 0)
      : isSnow ? Math.max(0, 0.4 + Math.sin(i / 8) * 0.2)
      : 0;
    const noise = (Math.sin(i * 1.3) + Math.cos(i * 0.7)) * 0.05;
    const intensity = Math.max(0, Math.min(1, baseI + noise));
    return {
      i, intensity,
      confHi: Math.min(1, intensity + 0.1 + i * 0.005),
      confLo: Math.max(0, intensity - 0.08 - i * 0.003),
    };
  });

  const nowcast = isRain ? {
    headline: 'Heavy rain in 12 min',
    sub: 'Lasts ~45 min · 1.2" possible · peak at +22m',
  } : isSnow ? {
    headline: 'Snow continuing next hour',
    sub: '~0.4" accumulation expected',
  } : null;

  const precipTotal = (hourly.reduce((s, h) => s + h.precip, 0) / 1000).toFixed(2);

  return {
    location: 'Cupertino, CA',
    timestamp: 'Updated 2 min ago',
    heroIcon: hero, timeOfDay: tweaks.timeOfDay,
    condition: isRain ? 'Heavy rain · wind gusts 30mph'
             : isClear ? (tweaks.timeOfDay === 'night' ? 'Clear · mild night' : 'Sunny · light breeze')
             : isSnow ? 'Snowing · 2" accumulation so far'
             : condition === 'storm' ? 'Thunderstorms · take cover'
             : condition === 'fog' ? 'Dense fog · reduced visibility'
             : 'Overcast · calm',
    temp, feels, hi: Math.max(...daily[0].hi ? [daily[0].hi] : [temp + 4]), lo: daily[0].lo,
    nowcast, hourly, daily, weekHi, weekLo, precipTotal,
    alertCount: condition === 'storm' ? 2 : isRain ? 1 : 0,
    uv: { value: isClear && tweaks.timeOfDay === 'day' ? 7 : 1, label: isClear ? 'High · burn 30m' : 'Low' },
    wind: { speed: isRain ? 18 : 7, dir: 'WSW', deg: 225 },
    humidity: isRain ? 92 : isClear ? 42 : 68, dew: isRain ? 52 : isClear ? 48 : 55,
    visibility: condition === 'fog' ? 1.2 : isRain ? 4 : 10,
    pressure: isRain ? 1003 : 1018, pressureTrend: isRain ? 'Falling fast' : 'Steady',
    aqi: { value: isRain ? 22 : isClear ? 38 : 55, label: isRain ? 'Good' : isClear ? 'Good' : 'Moderate' },
    sun: { rise: '6:42 AM', set: '7:58 PM', progress: 0.55 },
    nowcastFull: {
      location: 'Cupertino, CA',
      minutes,
      total: isRain ? 1.2 : isSnow ? 0.4 : 0.05,
      confidence: isRain ? 0.82 : 0.55,
      updatedAgo: '34 sec ago',
    },
  };
}

Object.assign(window, { App });

// Self-mount once DOM is ready
(function mount() {
  const el = document.getElementById('scale-wrap');
  if (!el) { setTimeout(mount, 30); return; }
  ReactDOM.createRoot(el).render(<App />);
})();

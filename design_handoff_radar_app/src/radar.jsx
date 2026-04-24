// Cumulus — Radar screen (Apple Weather-style polish)
// Full-bleed map · glass layer panel · left legend · bottom forecast pill · 1h/12h zoom toggle

const LAYERS = {
  precipitation: {
    name: 'Precipitation',
    legend: [
      ['Extreme', '#ffd74a'],
      ['Heavy',   '#7a3bff'],
      ['Moderate','#3b6dff'],
      ['Light',   '#7ec4ff'],
    ],
    unit: '',
  },
  temperature: {
    name: 'Temperature',
    legend: [
      ['130', '#8b1a3a'],
      ['90',  '#ff6e3a'],
      ['60',  '#7ae55f'],
      ['30',  '#4fb8ff'],
      ['0',   '#3a3fa8'],
      ['-40', '#1a0f4a'],
    ],
    unit: '°F',
  },
  air: {
    name: 'Air Quality',
    legend: [
      ['500', '#8B1A5B'],
      ['400', '#b03a66'],
      ['300', '#d4524e'],
      ['200', '#f59f3a'],
      ['100', '#f5d042'],
      ['0',   '#4ADE80'],
    ],
    unit: 'AQI',
  },
  wind: {
    name: 'Wind',
    legend: [
      ['75', '#ffffff'],
      ['50', '#c5dbff'],
      ['25', '#7ea8d9'],
      ['0',  '#4e78b5'],
    ],
    unit: 'mph',
  },
};

// Non-linear frame schedule: -60m → +24h
const FRAMES = (() => {
  const f = [];
  for (let m = -60; m < 0; m += 10) f.push(m);
  f.push(0);
  for (let m = 10; m <= 60; m += 10) f.push(m);
  for (let m = 90; m <= 360; m += 30) f.push(m);
  for (let m = 420; m <= 1440; m += 60) f.push(m);
  return f;
})();
const NOW_INDEX = FRAMES.indexOf(0);
const ZOOM_1H_START = FRAMES.indexOf(-60);
const ZOOM_1H_END = FRAMES.indexOf(60);

function formatClock(minuteOffset) {
  const base = new Date('2026-04-19T13:42:00');
  const d = new Date(base.getTime() + minuteOffset * 60000);
  const h = d.getHours(), m = d.getMinutes();
  const hr = h.toString().padStart(2, '0');
  const mn = m.toString().padStart(2, '0');
  return `${hr}:${mn}`;
}
function formatDay(minuteOffset) {
  const base = new Date('2026-04-19T13:42:00');
  const d = new Date(base.getTime() + minuteOffset * 60000);
  const days = ['Sunday','Monday','Tuesday','Wednesday','Thursday','Friday','Saturday'];
  const h = d.getHours().toString().padStart(2, '0');
  const m = d.getMinutes().toString().padStart(2, '0');
  return `${days[d.getDay()]} ${h}:${m}`;
}

function RadarScreen({ onBack, data }) {
  const t = window.CUMULUS.tokens;
  const f = window.CUMULUS.fonts;
  const [frame, setFrame] = React.useState(NOW_INDEX);
  const [playing, setPlaying] = React.useState(true);
  const [layerOpen, setLayerOpen] = React.useState(false);
  const [layer, setLayer] = React.useState('precipitation');
  const [zoom, setZoom] = React.useState('1h'); // '1h' | '12h'

  React.useEffect(() => {
    if (!playing) return;
    const id = setInterval(() => {
      setFrame(f => {
        const nextFrame = f + 1;
        const maxFrame = zoom === '1h' ? ZOOM_1H_END : FRAMES.length - 1;
        const minFrame = zoom === '1h' ? ZOOM_1H_START : 0;
        return nextFrame > maxFrame ? minFrame : nextFrame;
      });
    }, 420);
    return () => clearInterval(id);
  }, [playing, zoom]);

  const minuteOffset = FRAMES[frame];
  const isFuture = minuteOffset >= 0;
  const layerCfg = LAYERS[layer];

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', overflow: 'hidden', fontFamily: f.ui, color: '#0b1220', background: '#dbe8f3' }}>

      {/* Map + overlay */}
      <svg
        width="100%" height="100%" viewBox="0 0 390 770"
        preserveAspectRatio="xMidYMid slice"
        style={{ position: 'absolute', inset: 0 }}
      >
        <defs>
          <filter id="bigBlur" x="-30%" y="-30%" width="160%" height="160%">
            <feGaussianBlur stdDeviation="6" />
          </filter>
          <filter id="softBlur" x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur stdDeviation="3" />
          </filter>
          <linearGradient id="tempMap" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#4fb8ff" />
            <stop offset="0.35" stopColor="#7ae55f" />
            <stop offset="0.7" stopColor="#f5d042" />
            <stop offset="1" stopColor="#ff9f2e" />
          </linearGradient>
          <linearGradient id="aqMap" x1="0" y1="0" x2="0.3" y2="1">
            <stop offset="0" stopColor="#4ADE80" />
            <stop offset="1" stopColor="#7ee895" />
          </linearGradient>
          <pattern id="windStreaks" width="40" height="40" patternUnits="userSpaceOnUse" patternTransform="rotate(-22)">
            <line x1="0" y1="10" x2="24" y2="10" stroke="rgba(255,255,255,0.35)" strokeWidth="0.8" />
            <line x1="16" y1="24" x2="36" y2="24" stroke="rgba(255,255,255,0.45)" strokeWidth="0.7" />
            <line x1="4" y1="34" x2="20" y2="34" stroke="rgba(255,255,255,0.3)" strokeWidth="0.6" />
          </pattern>
        </defs>

        <MapBase layer={layer} />
        <LayerOverlay layer={layer} minuteOffset={minuteOffset} />
        <MapLabels layer={layer} />
        <UserLocation layer={layer} />
      </svg>

      {/* Status bar safe area (translucent top gradient for legibility) */}
      <div style={{ position: 'absolute', top: 0, left: 0, right: 0, height: 64, background: 'linear-gradient(180deg, rgba(255,255,255,0.35) 0%, rgba(255,255,255,0) 100%)', zIndex: 2, pointerEvents: 'none' }} />

      {/* Top row — close (left) · layer stack (right) */}
      <div style={{ position: 'absolute', top: 62, left: 16, zIndex: 6 }}>
        <CircleBtn onClick={onBack}>
          <svg width="14" height="14" viewBox="0 0 14 14"><path d="M3 3 L11 11 M11 3 L3 11" stroke="#0b1220" strokeWidth="1.8" strokeLinecap="round"/></svg>
        </CircleBtn>
      </div>

      <div style={{ position: 'absolute', top: 62, right: 16, zIndex: 6, display: 'flex', flexDirection: 'column', gap: 8 }}>
        <CircleBtn onClick={() => setLayerOpen(o => !o)} active={layerOpen}>
          <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
            <path d="M9 2 L15.5 5.5 L9 9 L2.5 5.5 Z" stroke="#0b1220" strokeWidth="1.4" strokeLinejoin="round" fill={layerOpen ? '#0b1220' : 'none'} fillOpacity={layerOpen ? 0.15 : 0}/>
            <path d="M2.5 9 L9 12.5 L15.5 9" stroke="#0b1220" strokeWidth="1.4" strokeLinejoin="round" opacity="0.65"/>
            <path d="M2.5 12.5 L9 16 L15.5 12.5" stroke="#0b1220" strokeWidth="1.4" strokeLinejoin="round" opacity="0.35"/>
          </svg>
        </CircleBtn>
        <CircleBtn>
          <svg width="18" height="18" viewBox="0 0 18 18"><path d="M9 1 L14 16 L9 12 L4 16 Z" fill="#0b1220"/></svg>
        </CircleBtn>
        <CircleBtn>
          <svg width="18" height="18" viewBox="0 0 18 18" fill="none"><circle cx="3" cy="5" r="0.8" fill="#0b1220"/><line x1="7" y1="5" x2="16" y2="5" stroke="#0b1220" strokeWidth="1.6" strokeLinecap="round"/><circle cx="3" cy="9" r="0.8" fill="#0b1220"/><line x1="7" y1="9" x2="16" y2="9" stroke="#0b1220" strokeWidth="1.6" strokeLinecap="round"/><circle cx="3" cy="13" r="0.8" fill="#0b1220"/><line x1="7" y1="13" x2="16" y2="13" stroke="#0b1220" strokeWidth="1.6" strokeLinecap="round"/></svg>
        </CircleBtn>
      </div>

      {/* Layer panel — glass, top right */}
      {layerOpen && (
        <div style={{
          position: 'absolute', top: 108, right: 16, width: 220, zIndex: 7,
          background: 'rgba(235,245,255,0.88)', backdropFilter: 'blur(28px) saturate(180%)',
          border: '0.5px solid rgba(255,255,255,0.6)',
          borderRadius: 18, padding: 6,
          boxShadow: '0 18px 40px rgba(20,30,60,0.22)',
        }}>
          {[
            ['precipitation', 'Precipitation', PrecipIcon],
            ['temperature',   'Temperature',   TempIcon],
            ['air',           'Air Quality',   AirIcon],
            ['wind',          'Wind',          WindIcon],
          ].map(([k, name, Icon]) => {
            const selected = layer === k;
            return (
              <div key={k} onClick={() => { setLayer(k); setLayerOpen(false); }}
                   style={{ display: 'flex', alignItems: 'center', padding: '10px 8px', borderRadius: 12, cursor: 'pointer', gap: 10 }}>
                <div style={{ width: 18, display: 'flex', justifyContent: 'center' }}>
                  {selected ? (
                    <svg width="13" height="12" viewBox="0 0 13 12"><path d="M1 6 L5 10 L12 1" stroke="#0b1220" strokeWidth="2" fill="none" strokeLinecap="round" strokeLinejoin="round"/></svg>
                  ) : null}
                </div>
                <Icon />
                <div style={{ fontSize: 17, color: '#0b1220', letterSpacing: -0.3 }}>{name}</div>
              </div>
            );
          })}
        </div>
      )}

      {/* Left legend — vertical gradient strip */}
      <div style={{
        position: 'absolute', left: 12, top: 116, width: 80, zIndex: 5,
        background: 'rgba(255,255,255,0.72)', backdropFilter: 'blur(24px)',
        border: '0.5px solid rgba(255,255,255,0.9)',
        borderRadius: 14, padding: '10px 10px 8px',
        boxShadow: '0 8px 24px rgba(20,30,60,0.12)',
      }}>
        <div style={{ fontSize: 11, fontWeight: 600, color: '#0b1220', marginBottom: 8 }}>
          {layerCfg.name}{layerCfg.unit ? ` (${layerCfg.unit})` : ''}
        </div>
        <div style={{ display: 'flex', gap: 6 }}>
          <div style={{
            width: 10, height: 96, borderRadius: 3,
            background: `linear-gradient(180deg, ${layerCfg.legend.map(l => l[1]).join(', ')})`,
          }} />
          <div style={{ display: 'flex', flexDirection: 'column', justifyContent: 'space-between', fontSize: 10, color: '#0b1220', lineHeight: 1 }}>
            {layerCfg.legend.map(([lbl]) => (<span key={lbl}>{lbl}</span>))}
          </div>
        </div>
        <div style={{ fontSize: 8.5, color: 'rgba(11,18,32,0.5)', marginTop: 8, textDecoration: 'underline' }}>Map Data</div>
      </div>

      {/* Bottom forecast pill */}
      <div style={{
        position: 'absolute', left: 10, right: 10, bottom: 44, zIndex: 6,
        background: 'rgba(255,255,255,0.82)', backdropFilter: 'blur(32px) saturate(180%)',
        border: '0.5px solid rgba(255,255,255,0.9)',
        borderRadius: 28, padding: '10px 14px',
        boxShadow: '0 12px 32px rgba(20,30,60,0.16)',
      }}>
        {/* header row */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <div onClick={() => setPlaying(p => !p)} style={{
            width: 30, height: 30, borderRadius: 15,
            background: 'rgba(11,18,32,0.08)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer', flexShrink: 0,
          }}>
            {playing
              ? <svg width="10" height="11" viewBox="0 0 10 11"><rect width="3" height="11" rx="1" fill="#0b1220"/><rect x="7" width="3" height="11" rx="1" fill="#0b1220"/></svg>
              : <svg width="10" height="11" viewBox="0 0 10 11"><path d="M0 0 L10 5.5 L0 11 Z" fill="#0b1220"/></svg>}
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: '#0b1220', letterSpacing: -0.2 }}>
              {minuteOffset === 0 ? 'Now' : isFuture ? 'Forecast' : 'Past'}
            </div>
            <div style={{ fontSize: 12, color: 'rgba(11,18,32,0.58)' }}>
              {formatDay(minuteOffset)}
            </div>
          </div>
          {/* 1h / 12h segmented */}
          <div style={{
            display: 'flex', background: 'rgba(11,18,32,0.08)', borderRadius: 14, padding: 2, height: 28,
          }}>
            {['1h','12h'].map(z => (
              <div key={z} onClick={() => {
                setZoom(z);
                if (z === '1h' && (frame < ZOOM_1H_START || frame > ZOOM_1H_END)) setFrame(NOW_INDEX);
              }}
              style={{
                padding: '0 12px', display: 'flex', alignItems: 'center', borderRadius: 12, cursor: 'pointer',
                background: zoom === z ? '#fff' : 'transparent',
                boxShadow: zoom === z ? '0 1px 3px rgba(0,0,0,0.14)' : 'none',
                fontSize: 13, fontWeight: 600, color: '#0b1220',
              }}>{z}</div>
            ))}
          </div>
        </div>

        {/* timeline */}
        <div style={{ marginTop: 10 }}>
          <TimelineTrack frame={frame} setFrame={setFrame} setPlaying={setPlaying} zoom={zoom} />
          <TimelineLabels zoom={zoom} frame={frame} />
        </div>
      </div>
    </div>
  );
}

// ── Map base ── bright Apple-maps style
function MapBase({ layer }) {
  // Base land + water colors vary by layer (temp/air use the data as base)
  const baseLand = layer === 'temperature' ? 'url(#tempMap)'
                  : layer === 'air' ? 'url(#aqMap)'
                  : layer === 'wind' ? '#6b8cb5'
                  : '#e6efe8';
  const water = layer === 'temperature' ? '#3a7fbf'
              : layer === 'air' ? '#b8e2c4'
              : layer === 'wind' ? '#3b5d85'
              : '#c6dbe8';
  const roadColor = layer === 'wind' || layer === 'temperature' ? 'rgba(255,255,255,0.25)' : 'rgba(11,18,32,0.15)';
  const stateLineColor = layer === 'wind' || layer === 'temperature' ? 'rgba(255,255,255,0.3)' : 'rgba(11,18,32,0.22)';

  return (
    <g>
      <rect width="390" height="770" fill={baseLand} />

      {/* "Great Lakes" style water shapes for familiarity */}
      <path d="M 60 130 Q 30 150 50 200 Q 90 230 140 200 Q 180 180 160 140 Q 120 110 60 130 Z" fill={water} />
      <path d="M 200 100 Q 170 130 190 180 Q 220 220 260 200 Q 290 170 270 120 Q 240 90 200 100 Z" fill={water} />
      <path d="M 280 180 Q 260 230 300 270 Q 340 280 370 250 Q 380 210 350 180 Q 310 160 280 180 Z" fill={water} />
      <path d="M -20 280 Q 0 320 50 310 Q 90 290 110 320 L 110 380 Q 70 400 30 380 Q -10 360 -20 320 Z" fill={water} />

      {/* Long river */}
      <path d="M 100 400 Q 140 440 160 500 Q 180 580 210 640" stroke={water} strokeWidth="4" fill="none" strokeLinecap="round" opacity="0.8"/>

      {/* State boundaries */}
      <g stroke={stateLineColor} strokeWidth="0.6" fill="none">
        <path d="M 0 350 L 390 350" />
        <path d="M 0 520 L 390 520" />
        <path d="M 140 0 L 140 350" />
        <path d="M 140 350 L 260 350 L 260 520" />
        <path d="M 260 150 L 260 350" />
        <path d="M 60 520 L 60 770" />
        <path d="M 180 520 L 180 770" />
        <path d="M 300 520 L 300 770" />
      </g>

      {/* Road network */}
      <g stroke={roadColor} strokeWidth="0.7" fill="none" strokeLinecap="round">
        <path d="M 20 430 Q 150 460 240 500 T 390 560" />
        <path d="M 0 600 Q 100 590 180 610 T 390 650" />
        <path d="M 100 350 Q 140 420 180 520 T 220 770" />
        <path d="M 260 350 L 300 520 L 320 770" />
        <path d="M 0 500 Q 80 520 140 520" />
      </g>
      {/* Major highways */}
      <g stroke={layer === 'wind' || layer === 'temperature' ? 'rgba(255,255,255,0.45)' : 'rgba(11,18,32,0.22)'} strokeWidth="1.4" fill="none" strokeLinecap="round">
        <path d="M 20 430 Q 150 460 240 500 T 390 560" />
        <path d="M 100 350 Q 140 420 180 520 T 220 770" />
      </g>
    </g>
  );
}

// ── Layer overlay ── animated data on top of base map
function LayerOverlay({ layer, minuteOffset }) {
  if (layer === 'precipitation') return <PrecipOverlay minuteOffset={minuteOffset} />;
  if (layer === 'wind') return <WindOverlay minuteOffset={minuteOffset} />;
  // temperature + air use the base map gradients; add subtle banding
  if (layer === 'air') {
    // soft yellow/orange pollution patch moves slowly
    const x = 60 + (minuteOffset / 60) * 3;
    return (
      <g opacity="0.5">
        <ellipse cx={280 + x * 0.2} cy={720} rx="180" ry="80" fill="#f5d042" filter="url(#bigBlur)" />
        <ellipse cx={340} cy={750} rx="120" ry="70" fill="#ff9f2e" filter="url(#bigBlur)" opacity="0.6" />
      </g>
    );
  }
  if (layer === 'temperature') {
    // southern warm patch
    return (
      <g opacity="0.45">
        <ellipse cx={320} cy={700} rx="180" ry="120" fill="#ff6e3a" filter="url(#bigBlur)" />
        <ellipse cx={80}  cy={100} rx="140" ry="90"  fill="#4fb8ff" filter="url(#bigBlur)" />
      </g>
    );
  }
  return null;
}

function PrecipOverlay({ minuteOffset }) {
  const cells = [
    { cx: 40,  cy: 180, r: 44, i: 0.9, vx: 0.6, vy: 0.2 },
    { cx: 120, cy: 220, r: 30, i: 0.7, vx: 0.5, vy: 0.4 },
    { cx: 200, cy: 280, r: 22, i: 0.5, vx: 0.4, vy: 0.3 },
    { cx: 300, cy: 160, r: 35, i: 0.6, vx: 0.5, vy: 0.3 },
    { cx: 250, cy: 340, r: 26, i: 0.55, vx: 0.6, vy: 0.2 },
    { cx: 80,  cy: 560, r: 18, i: 0.4, vx: 0.7, vy: 0.1 },
    { cx: 340, cy: 500, r: 14, i: 0.35, vx: 0.3, vy: 0.3 },
  ];
  return (
    <g>
      {cells.map((c, i) => {
        const dx = (minuteOffset / 60) * c.vx * 40;
        const dy = (minuteOffset / 60) * c.vy * 40;
        const hrs = Math.abs(minuteOffset) / 60;
        const fade = Math.max(0.35, 1 - hrs / 18);
        const bands = [
          { r: c.r * 2.2, color: '#7ec4ff', op: 0.22 * c.i * fade },
          { r: c.r * 1.7, color: '#3b6dff', op: 0.35 * c.i * fade },
          { r: c.r * 1.2, color: '#7a3bff', op: 0.55 * c.i * fade },
          { r: c.r * 0.7, color: '#ffd74a', op: 0.7 * c.i * fade },
        ];
        return bands.map((b, j) => (
          <ellipse key={`${i}-${j}`} cx={c.cx + dx} cy={c.cy + dy} rx={b.r} ry={b.r * 0.75} fill={b.color} opacity={b.op} filter="url(#softBlur)" />
        ));
      })}
    </g>
  );
}

function WindOverlay({ minuteOffset }) {
  const shift = (minuteOffset / 60) * 8;
  return (
    <g>
      {/* Moving streaks — background overlay feels like velocity */}
      <rect width="390" height="770" fill="url(#windStreaks)" transform={`translate(${shift},0)`} opacity="0.7"/>
      <rect width="390" height="770" fill="url(#windStreaks)" transform={`translate(${-shift*0.6},10) rotate(4)`} opacity="0.35"/>
      {/* High wind patches */}
      <g opacity="0.55" style={{ mixBlendMode: 'screen' }}>
        <ellipse cx={70 + shift * 0.4} cy={220} rx="80" ry="50" fill="#c5dbff" filter="url(#bigBlur)" />
        <ellipse cx={280 - shift * 0.2} cy={160} rx="60" ry="40" fill="#e8f0ff" filter="url(#bigBlur)" />
      </g>
    </g>
  );
}

// ── Map labels
function MapLabels({ layer }) {
  const light = layer === 'wind' || layer === 'temperature';
  const cities = [
    { x: 60,  y: 440, name: 'Chicago',       major: true },
    { x: 22,  y: 420, name: 'Rockford',      minor: true },
    { x: 120, y: 440, name: 'South Bend',    minor: true },
    { x: 180, y: 435, name: 'Fort Wayne',    minor: true },
    { x: 150, y: 540, name: 'Indianapolis',  major: true },
    { x: 215, y: 555, name: 'Dayton',        minor: true },
    { x: 265, y: 555, name: 'Columbus',      major: true },
    { x: 235, y: 445, name: 'Toledo',        minor: true },
    { x: 285, y: 445, name: 'Cleveland',     minor: true },
    { x: 310, y: 430, name: 'Akron',         minor: true },
    { x: 375, y: 455, name: 'Pittsburgh',    minor: true },
    { x: 215, y: 420, name: 'Detroit',       major: true },
    { x: 180, y: 420, name: 'Lansing',       minor: true },
    { x: 160, y: 400, name: 'Flint',         minor: true },
    { x: 290, y: 400, name: 'London',        minor: true },
    { x: 80,  y: 370, name: 'Milwaukee',     minor: true },
    { x: 80,  y: 340, name: 'Madison',       minor: true },
    { x: 130, y: 330, name: 'Green Bay',     minor: true },
    { x: 165, y: 620, name: 'Cincinnati',    minor: true },
    { x: 175, y: 670, name: 'Louisville',    minor: true },
    { x: 245, y: 670, name: 'Lexington',     minor: true },
  ];
  const regions = [
    { x: 140, y: 380, label: 'MI' },
    { x: 230, y: 495, label: 'OH' },
    { x: 155, y: 505, label: 'IN' },
    { x: 50,  y: 385, label: 'WI' },
    { x: 40,  y: 495, label: 'IL' },
    { x: 210, y: 720, label: 'KY' },
    { x: 370, y: 500, label: 'WV' },
  ];
  const waters = [
    { x: 85,  y: 165, label: 'Lake Michigan', italic: true },
    { x: 240, y: 140, label: 'Lake Huron', italic: true },
    { x: 320, y: 225, label: 'Lake Erie', italic: true },
  ];

  const textFill = light ? '#f5faff' : '#0b1220';
  const strokeFill = light ? 'rgba(30,50,90,0.55)' : 'rgba(255,255,255,0.92)';

  return (
    <g fontFamily="-apple-system, SF Pro" style={{ paintOrder: 'stroke' }}>
      {regions.map((r, i) => (
        <text key={'r'+i} x={r.x} y={r.y} fontSize="11" fontWeight="700" fill={light ? 'rgba(255,255,255,0.55)' : 'rgba(11,18,32,0.35)'} textAnchor="middle" stroke={strokeFill} strokeWidth="2.5">{r.label}</text>
      ))}
      {waters.map((w, i) => (
        <text key={'w'+i} x={w.x} y={w.y} fontSize="9" fontStyle="italic" fill={light ? 'rgba(255,255,255,0.65)' : 'rgba(11,50,90,0.55)'} textAnchor="middle" stroke={strokeFill} strokeWidth="2">{w.label}</text>
      ))}
      {cities.map((c, i) => (
        <g key={'c'+i}>
          <circle cx={c.x} cy={c.y} r="1.5" fill={textFill} />
          <text x={c.x + 4} y={c.y + 3.5}
                fontSize={c.major ? 11 : 9.5}
                fontWeight={c.major ? 700 : 500}
                fill={textFill}
                stroke={strokeFill} strokeWidth={c.major ? 2.6 : 2}
                strokeLinejoin="round">
            {c.name}
          </text>
        </g>
      ))}
    </g>
  );
}

// ── User location pill with reading value
function UserLocation({ layer }) {
  const cx = 170, cy = 430;
  // reading varies by layer
  const reading = {
    precipitation: { top: '', mid: '45°', bot: '' },
    temperature:   { top: '', mid: '45', bot: '°F' },
    air:           { top: '', mid: '35', bot: 'AQI' },
    wind:          { top: 'W', mid: '15', bot: 'MPH' },
  }[layer];
  const onDark = layer === 'wind' || layer === 'temperature';
  return (
    <g>
      {/* Pulse */}
      <circle cx={cx} cy={cy} r="28" fill="#fff" opacity="0.25">
        <animate attributeName="r" values="22;34;22" dur="2.4s" repeatCount="indefinite"/>
        <animate attributeName="opacity" values="0.4;0.08;0.4" dur="2.4s" repeatCount="indefinite"/>
      </circle>
      {/* Pill */}
      <g>
        <circle cx={cx} cy={cy} r="19" fill="#fff" stroke="rgba(11,18,32,0.18)" strokeWidth="0.8" />
        {reading.top && <text x={cx} y={cy - 6} fontSize="8" fontWeight="700" fill="#0b1220" textAnchor="middle" fontFamily="-apple-system, SF Pro">{reading.top}</text>}
        <text x={cx} y={cy + (reading.top ? 3 : 4)} fontSize="14" fontWeight="700" fill={layer === 'precipitation' ? '#0b1220' : '#0b1220'} textAnchor="middle" fontFamily="-apple-system, SF Pro">
          {reading.mid}
        </text>
        {reading.bot && <text x={cx} y={cy + 13} fontSize="6.5" fontWeight="700" fill="rgba(11,18,32,0.7)" textAnchor="middle" fontFamily="-apple-system, SF Pro">{reading.bot}</text>}
      </g>
      {/* "My Location" tag */}
      <text x={cx + 24} y={cy + 4} fontSize="10" fontWeight="600" fill={onDark ? '#fff' : '#0b1220'} stroke={onDark ? 'rgba(30,50,90,0.55)' : 'rgba(255,255,255,0.92)'} strokeWidth="2" style={{ paintOrder: 'stroke' }} fontFamily="-apple-system, SF Pro">My Location</text>
    </g>
  );
}

// ── Timeline
function TimelineTrack({ frame, setFrame, setPlaying, zoom }) {
  const trackRef = React.useRef(null);
  const startIdx = zoom === '1h' ? ZOOM_1H_START : 0;
  const endIdx   = zoom === '1h' ? ZOOM_1H_END   : FRAMES.length - 1;
  const nowIdx   = NOW_INDEX;
  const len      = endIdx - startIdx;

  const handleDrag = (e) => {
    const rect = trackRef.current.getBoundingClientRect();
    const clientX = e.clientX ?? (e.touches && e.touches[0].clientX);
    const x = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
    setFrame(startIdx + Math.round(x * len));
  };

  // Tick set
  const ticks = [];
  if (zoom === '1h') {
    for (let m = -60; m <= 60; m += 5) {
      const i = FRAMES.indexOf(m);
      if (i === -1) continue;
      const major = m % 30 === 0;
      ticks.push({ i, major });
    }
  } else {
    for (let h = -1; h <= 24; h++) {
      const m = h * 60;
      const i = FRAMES.indexOf(m);
      if (i === -1) continue;
      const major = h % 3 === 0;
      ticks.push({ i, major });
    }
  }

  const pct = (i) => ((i - startIdx) / len) * 100;
  const thumbPct = Math.max(0, Math.min(100, pct(frame)));
  const nowPct = pct(nowIdx);

  return (
    <div
      ref={trackRef}
      onMouseDown={(e) => {
        setPlaying(false); handleDrag(e);
        const up = () => { document.removeEventListener('mousemove', handleDrag); document.removeEventListener('mouseup', up); };
        document.addEventListener('mousemove', handleDrag); document.addEventListener('mouseup', up);
      }}
      style={{ position: 'relative', height: 22, cursor: 'pointer' }}
    >
      {/* full bar */}
      <div style={{ position: 'absolute', left: 0, right: 0, top: 10, height: 3, background: 'rgba(11,18,32,0.12)', borderRadius: 2 }} />
      {/* played (past to now) */}
      {nowPct > 0 && (
        <div style={{ position: 'absolute', left: 0, width: `${Math.min(thumbPct, nowPct)}%`, top: 10, height: 3, background: 'rgba(11,18,32,0.55)', borderRadius: 2 }} />
      )}
      {/* ticks */}
      {ticks.map(({ i, major }) => (
        <div key={i} style={{
          position: 'absolute', left: `${pct(i)}%`, top: major ? 7 : 9,
          width: 1, height: major ? 9 : 5,
          background: i === nowIdx ? 'transparent' : 'rgba(11,18,32,0.22)',
          transform: 'translateX(-50%)',
        }} />
      ))}
      {/* Now marker */}
      {nowPct >= 0 && nowPct <= 100 && (
        <div style={{ position: 'absolute', left: `${nowPct}%`, top: 4, width: 2, height: 14, background: '#0b1220', transform: 'translateX(-50%)', borderRadius: 1 }} />
      )}
      {/* Thumb */}
      <div style={{
        position: 'absolute', left: `${thumbPct}%`, top: 3,
        width: 16, height: 16, borderRadius: 8,
        background: '#fff', border: '1.5px solid #0b1220',
        transform: 'translateX(-50%)',
        boxShadow: '0 2px 6px rgba(0,0,0,0.2)',
      }} />
    </div>
  );
}

function TimelineLabels({ zoom, frame }) {
  let labels;
  if (zoom === '1h') {
    labels = [];
    for (let m = -60; m <= 60; m += 15) {
      const i = FRAMES.indexOf(m);
      if (i === -1) continue;
      labels.push({ i, label: m === 0 ? 'Now' : formatClock(m) });
    }
  } else {
    labels = [];
    for (let h = 0; h <= 24; h += 3) {
      const m = h * 60;
      const i = FRAMES.indexOf(m);
      if (i === -1) continue;
      labels.push({ i, label: h === 0 ? 'Now' : formatClock(m) });
    }
    // Insert -1h
    labels.unshift({ i: ZOOM_1H_START, label: formatClock(-60) });
  }

  const startIdx = zoom === '1h' ? ZOOM_1H_START : 0;
  const endIdx   = zoom === '1h' ? ZOOM_1H_END   : FRAMES.length - 1;
  const len      = endIdx - startIdx;

  return (
    <div style={{ position: 'relative', height: 16, marginTop: 4 }}>
      {labels.map(({ i, label }) => {
        const pct = ((i - startIdx) / len) * 100;
        if (pct < -2 || pct > 102) return null;
        const isNow = label === 'Now';
        return (
          <span key={i} style={{
            position: 'absolute', left: `${Math.max(0, Math.min(100, pct))}%`,
            transform: 'translateX(-50%)',
            fontSize: 10.5, fontWeight: isNow ? 700 : 500,
            color: isNow ? '#0b1220' : 'rgba(11,18,32,0.55)',
            whiteSpace: 'nowrap',
          }}>{label}</span>
        );
      })}
    </div>
  );
}

// ── Circular glass button
function CircleBtn({ children, onClick, active }) {
  return (
    <div onClick={onClick} style={{
      width: 40, height: 40, borderRadius: 20,
      background: active ? 'rgba(255,255,255,0.98)' : 'rgba(255,255,255,0.78)',
      backdropFilter: 'blur(24px) saturate(180%)',
      border: '0.5px solid rgba(255,255,255,0.9)',
      boxShadow: '0 4px 14px rgba(20,30,60,0.14)',
      display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer',
    }}>{children}</div>
  );
}

// ── Layer menu icons
function PrecipIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" fill="none">
      <path d="M5 11 C3 11 2 9.5 2 8 C2 6 4 5 5.5 5.5 C6 3.5 8.5 2.5 11 3 C13.5 3.5 15 5 15 7 C17 7 18 8.5 18 10 C18 11 17 12 16 12" stroke="#0b1220" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round"/>
      <line x1="7" y1="14" x2="6" y2="18" stroke="#3b6dff" strokeWidth="1.6" strokeLinecap="round"/>
      <line x1="11" y1="14" x2="10" y2="19" stroke="#3b6dff" strokeWidth="1.6" strokeLinecap="round"/>
      <line x1="15" y1="14" x2="14" y2="18" stroke="#3b6dff" strokeWidth="1.6" strokeLinecap="round"/>
    </svg>
  );
}
function TempIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" fill="none">
      <path d="M10 4 C10 3 10.5 2.5 11 2.5 C11.5 2.5 12 3 12 4 V13.5" stroke="#0b1220" strokeWidth="1.3" strokeLinecap="round"/>
      <path d="M12 13.5 A3 3 0 1 1 10 13.5" stroke="#0b1220" strokeWidth="1.3" strokeLinecap="round"/>
      <circle cx="11" cy="16" r="1.6" fill="#ff6e3a"/>
      <line x1="11" y1="14" x2="11" y2="6" stroke="#ff6e3a" strokeWidth="1.4" strokeLinecap="round"/>
    </svg>
  );
}
function AirIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" fill="none">
      {[[5,5],[9,4],[13,5],[17,5],[4,9],[8,9],[12,9],[16,9],[5,13],[9,13],[13,13],[17,13],[6,17],[10,17],[14,17]].map(([x,y],i)=> (
        <circle key={i} cx={x} cy={y} r="0.9" fill="#0b1220"/>
      ))}
    </svg>
  );
}
function WindIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" fill="none">
      <path d="M3 7 H13 C14.5 7 15.5 5.8 15.5 4.5 C15.5 3.3 14.5 2.5 13.5 2.5 C12.5 2.5 11.8 3.2 11.8 4" stroke="#0b1220" strokeWidth="1.3" strokeLinecap="round"/>
      <path d="M3 11 H16.5 C18 11 19 9.8 19 8.5 C19 7.3 18 6.5 17 6.5 C16 6.5 15.3 7.2 15.3 8" stroke="#0b1220" strokeWidth="1.3" strokeLinecap="round"/>
      <path d="M3 15 H12 C13 15 13.8 15.8 13.8 17 C13.8 18 13 18.5 12.3 18.5 C11.5 18.5 11 18 11 17.4" stroke="#0b1220" strokeWidth="1.3" strokeLinecap="round"/>
    </svg>
  );
}

Object.assign(window, { RadarScreen });

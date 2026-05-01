// Radar-NG — Home screen
// Carrot-faithful: hero temp + condition, hourly strip (24h), 7-day, nowcast banner,
// plus dense data cards (UV, wind, sun, precip, humidity, visibility).

function HomeScreen({ data, onOpenRadar, onOpenNowcast }) {
  const t = window.CUMULUS.tokens;
  const f = window.CUMULUS.fonts;

  return (
    <PullToRefresh accent="#8B7CFF" onRefresh={() => new Promise(r => setTimeout(r, 600))}>
    <div style={{
      width: '100%', minHeight: '100%',
      fontFamily: f.ui, color: t.ink,
      paddingBottom: 100, paddingTop: 56,
    }}>
      {/* Top bar — location */}
      <div style={{ padding: '8px 20px 4px', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 15, fontWeight: 600 }}>
            <LocationDot /> {data.location}
          </div>
          <div style={{ fontSize: 11, color: t.inkMuted, fontFamily: f.mono, letterSpacing: '0.04em', marginTop: 2 }}>
            {data.timestamp}
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <IconBtn><SearchIcon /></IconBtn>
          <IconBtn><MenuIcon /></IconBtn>
        </div>
      </div>

      {/* Hero temp + big illustration */}
      <div style={{ padding: '8px 20px 0', position: 'relative', minHeight: 200 }}>
        <div style={{ position: 'absolute', right: 14, top: -6, opacity: 0.95 }}>
          <WeatherIcon kind={data.heroIcon} size={140} time={data.timeOfDay} />
        </div>
        <div style={{ fontSize: 13, color: t.inkDim, fontWeight: 500 }}>{data.condition}</div>
        <div style={{
          fontFamily: f.display, fontWeight: 200,
          fontSize: 112, lineHeight: 1, letterSpacing: '-0.055em',
          marginTop: 6,
          display: 'flex', alignItems: 'flex-start',
        }}>
          {data.temp}<span style={{ fontSize: 56, fontWeight: 300, marginTop: 12, letterSpacing: '-0.03em', opacity: 0.55 }}>°</span>
        </div>
        <div style={{ fontSize: 14, color: t.inkDim, marginTop: -4 }}>
          Feels like <span style={{ color: t.ink, fontWeight: 500 }}>{data.feels}°</span>
          <span style={{ margin: '0 8px', color: t.inkFaint }}>·</span>
          H <span style={{ color: t.ink, fontWeight: 500 }}>{data.hi}°</span>
          <span style={{ margin: '0 6px', color: t.inkFaint }}>L</span>
          <span style={{ color: t.ink, fontWeight: 500 }}>{data.lo}°</span>
        </div>
      </div>

      {/* Nowcast banner — clickable */}
      {data.nowcast && (
        <div onClick={onOpenNowcast}
             style={{
          margin: '18px 16px 0', padding: '12px 14px',
          background: `linear-gradient(90deg, rgba(79,184,255,0.22), rgba(30,127,255,0.10))`,
          border: '1px solid rgba(79,184,255,0.45)',
          borderRadius: 16, cursor: 'pointer',
          display: 'flex', alignItems: 'center', gap: 12,
        }}>
          <div style={{ width: 38, height: 38, borderRadius: 12, background: 'rgba(79,184,255,0.3)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <WeatherIcon kind="rain" size={28} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 14, fontWeight: 600 }}>{data.nowcast.headline}</div>
            <div style={{ fontSize: 12, color: t.inkDim, marginTop: 1 }}>{data.nowcast.sub}</div>
          </div>
          <Chevron />
        </div>
      )}

      {/* 24-hour hourly strip */}
      <SectionHeader title="HOURLY · NEXT 24" right="48H" />
      <div style={{ padding: '0 4px 0 12px' }}>
        <div style={{
          display: 'flex', gap: 8, overflowX: 'auto', padding: '0 8px 6px',
          scrollbarWidth: 'none',
        }}>
          {data.hourly.map((h, i) => <HourlyCell key={i} h={h} isNow={i === 0} />)}
        </div>
      </div>

      {/* Precip chance micro-chart */}
      <SectionHeader title="PRECIPITATION · 24H" right={`${data.precipTotal}"`}/>
      <div style={{ margin: '0 16px', background: t.card, border: `1px solid ${t.cardLine}`, borderRadius: 18, padding: '14px 16px 12px' }}>
        <PrecipChart hourly={data.hourly} />
        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: t.inkMuted, fontFamily: f.mono, marginTop: 6 }}>
          <span>NOW</span><span>+6h</span><span>+12h</span><span>+18h</span><span>+24h</span>
        </div>
      </div>

      {/* 7-day */}
      <SectionHeader title="7 DAY FORECAST" right="" />
      <div style={{ margin: '0 16px', background: t.card, border: `1px solid ${t.cardLine}`, borderRadius: 18, overflow: 'hidden' }}>
        {data.daily.map((d, i) => <DailyRow key={i} d={d} globalMin={data.weekLo} globalMax={data.weekHi} isFirst={i === 0} />)}
      </div>

      {/* Radar tease */}
      <div onClick={onOpenRadar} style={{
        margin: '18px 16px 0', borderRadius: 18, overflow: 'hidden', cursor: 'pointer',
        background: t.card, border: `1px solid ${t.cardLine}`, position: 'relative', height: 140,
      }}>
        <RadarMiniPreview />
        <div style={{ position: 'absolute', inset: 0, background: 'linear-gradient(180deg, transparent 40%, rgba(0,0,0,0.65))', pointerEvents: 'none' }} />
        <div style={{ position: 'absolute', left: 14, right: 14, bottom: 12, display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div>
            <div style={{ fontSize: 10, fontFamily: f.mono, letterSpacing: '0.12em', color: 'rgba(255,255,255,0.65)' }}>RADAR</div>
            <div style={{ fontSize: 15, fontWeight: 600 }}>Storm cell 4.2 mi NE · closing</div>
          </div>
          <div style={{ width: 30, height: 30, borderRadius: 10, background: 'rgba(255,255,255,0.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Chevron />
          </div>
        </div>
        <div style={{ position: 'absolute', top: 10, left: 12, display: 'flex', alignItems: 'center', gap: 5, fontSize: 10, fontFamily: f.mono, color: '#4ADE80', fontWeight: 600, letterSpacing: '0.1em' }}>
          <span style={{ width: 6, height: 6, borderRadius: 3, background: '#4ADE80', boxShadow: '0 0 6px #4ADE80' }} />
          LIVE
        </div>
      </div>

      {/* Stat grid */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, margin: '12px 16px 0' }}>
        <StatCard label="UV INDEX" value={data.uv.value} sub={data.uv.label} viz={<UVBar value={data.uv.value} />} />
        <StatCard label="WIND" value={data.wind.speed} unit="mph" sub={data.wind.dir} viz={<WindDial dir={data.wind.deg} />} />
        <StatCard label="HUMIDITY" value={data.humidity} unit="%" sub={`Dew ${data.dew}°`} viz={<FillRing value={data.humidity / 100} color={window.CUMULUS.tokens.rain} />} />
        <StatCard label="VISIBILITY" value={data.visibility} unit="mi" sub="Clear" viz={<VisBars value={data.visibility} />} />
        <StatCard label="PRESSURE" value={data.pressure} unit="hPa" sub={data.pressureTrend} viz={<PressureGauge value={data.pressure} />} />
        <StatCard label="AIR QUALITY" value={data.aqi.value} sub={data.aqi.label} viz={<AQIBar value={data.aqi.value} />} />
      </div>

      {/* Sun + moon */}
      <SectionHeader title="SUN & MOON" right="" />
      <div style={{ margin: '0 16px 20px', background: window.CUMULUS.tokens.card, border: `1px solid ${window.CUMULUS.tokens.cardLine}`, borderRadius: 18, padding: '16px' }}>
        <SunArc sunrise={data.sun.rise} sunset={data.sun.set} progress={data.sun.progress} />
      </div>
    </div>
    </PullToRefresh>
  );
}

// ───── sub-components

function SectionHeader({ title, right }) {
  const t = window.CUMULUS.tokens, f = window.CUMULUS.fonts;
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', padding: '22px 20px 10px' }}>
      <div style={{ fontSize: 11, fontFamily: f.mono, color: t.inkMuted, letterSpacing: '0.14em', fontWeight: 600 }}>{title}</div>
      {right && <div style={{ fontSize: 11, fontFamily: f.mono, color: t.inkDim, letterSpacing: '0.08em' }}>{right}</div>}
    </div>
  );
}

function HourlyCell({ h, isNow }) {
  const t = window.CUMULUS.tokens, f = window.CUMULUS.fonts;
  return (
    <div style={{
      minWidth: 54, padding: '10px 4px 8px', textAlign: 'center',
      background: isNow ? 'rgba(139,124,255,0.18)' : t.card,
      border: isNow ? '1px solid rgba(139,124,255,0.55)' : `1px solid ${t.cardLine}`,
      borderRadius: 14,
    }}>
      <div style={{ fontSize: 10, fontFamily: f.mono, color: isNow ? '#C7BDFF' : t.inkMuted, fontWeight: 600, letterSpacing: '0.04em' }}>
        {isNow ? 'NOW' : h.time}
      </div>
      <div style={{ margin: '4px 0' }}>
        <WeatherIcon kind={h.icon} size={30} time={h.timeOfDay} />
      </div>
      <div style={{ fontSize: 15, fontWeight: 600 }}>{h.temp}°</div>
      {h.precip > 0 && (
        <div style={{ fontSize: 9, color: t.rain, fontFamily: f.mono, fontWeight: 600, marginTop: 2 }}>
          {h.precip}%
        </div>
      )}
    </div>
  );
}

function DailyRow({ d, globalMin, globalMax, isFirst }) {
  const t = window.CUMULUS.tokens, f = window.CUMULUS.fonts;
  const range = globalMax - globalMin;
  const leftPct = ((d.lo - globalMin) / range) * 100;
  const widthPct = ((d.hi - d.lo) / range) * 100;
  return (
    <div style={{
      display: 'grid',
      gridTemplateColumns: '50px 34px 44px 1fr 44px',
      alignItems: 'center', gap: 10,
      padding: '12px 14px',
      borderTop: isFirst ? 'none' : `1px solid ${t.cardLine}`,
    }}>
      <div style={{ fontSize: 14, fontWeight: 600, color: isFirst ? t.ink : t.inkDim }}>
        {isFirst ? 'Today' : d.day}
      </div>
      <div><WeatherIcon kind={d.icon} size={28} /></div>
      <div style={{ fontSize: 12, color: t.rain, fontFamily: f.mono, fontWeight: 600, textAlign: 'right' }}>
        {d.precip > 0 ? `${d.precip}%` : ''}
      </div>
      <div style={{ position: 'relative', height: 6 }}>
        {/* track */}
        <div style={{ position: 'absolute', left: 0, right: 0, top: 2, height: 3, borderRadius: 2, background: 'rgba(255,255,255,0.08)' }} />
        {/* range bar */}
        <div style={{
          position: 'absolute', left: `${leftPct}%`, width: `${widthPct}%`, top: 0, height: 6, borderRadius: 3,
          background: `linear-gradient(90deg, ${t.cold} 0%, ${t.sun} 50%, ${t.hot} 100%)`,
        }} />
        {/* current */}
        {isFirst && (
          <div style={{
            position: 'absolute', left: `${((d.now - globalMin) / range) * 100}%`,
            top: -2, width: 10, height: 10, borderRadius: 5,
            background: '#fff', border: '2px solid #000', transform: 'translateX(-50%)',
          }} />
        )}
      </div>
      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', fontSize: 13, fontFamily: f.mono, fontWeight: 500 }}>
        <span style={{ color: t.inkDim, minWidth: 18, textAlign: 'right' }}>{d.lo}°</span>
        <span style={{ minWidth: 18, textAlign: 'right' }}>{d.hi}°</span>
      </div>
    </div>
  );
}

function StatCard({ label, value, unit, sub, viz }) {
  const t = window.CUMULUS.tokens, f = window.CUMULUS.fonts;
  return (
    <div style={{
      background: t.card, border: `1px solid ${t.cardLine}`, borderRadius: 16,
      padding: '11px 13px 12px', position: 'relative', minHeight: 92,
    }}>
      <div style={{ fontSize: 10, fontFamily: f.mono, color: t.inkMuted, letterSpacing: '0.12em', fontWeight: 600 }}>{label}</div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 4, marginTop: 4 }}>
        <span style={{ fontSize: 24, fontWeight: 500, letterSpacing: '-0.02em' }}>{value}</span>
        {unit && <span style={{ fontSize: 12, color: t.inkDim }}>{unit}</span>}
      </div>
      <div style={{ fontSize: 11, color: t.inkDim, marginTop: 2 }}>{sub}</div>
      <div style={{ position: 'absolute', right: 10, bottom: 10 }}>{viz}</div>
    </div>
  );
}

function PrecipChart({ hourly }) {
  const t = window.CUMULUS.tokens;
  const bars = hourly.slice(0, 24);
  return (
    <div style={{ display: 'flex', alignItems: 'flex-end', gap: 2, height: 48, position: 'relative' }}>
      {bars.map((h, i) => {
        const pct = h.precip / 100;
        const h1 = Math.max(2, pct * 42);
        return (
          <div key={i} style={{ flex: 1, height: 48, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', gap: 1 }}>
            <div style={{ height: h1, background: `linear-gradient(180deg, ${t.rainHeavy}, ${t.rain})`, borderRadius: 2, opacity: pct > 0.05 ? 1 : 0.25 }} />
          </div>
        );
      })}
    </div>
  );
}

// Mini radar preview (static-ish SVG)
function RadarMiniPreview() {
  // Realistic squall-line signature, scaled-down version of full radar field
  const W = 360, H = 140;
  const GX = 60, GY = 22;
  const dx = W / GX, dy = H / GY;

  function hash2(x, y, s){ const h=Math.sin(x*12.9898+y*78.233+s*43.758)*43758.5453; return h-Math.floor(h); }
  function smooth(t){ return t*t*(3-2*t); }
  function vnoise(x, y, s){ const xi=Math.floor(x), yi=Math.floor(y); const xf=x-xi, yf=y-yi; const a=hash2(xi,yi,s), b=hash2(xi+1,yi,s), c=hash2(xi,yi+1,s), d=hash2(xi+1,yi+1,s); const u=smooth(xf), v=smooth(yf); return a+(b-a)*u+(c-a)*v+(a-b-c+d)*u*v; }
  function fbm(x, y, s){ let v=0, amp=0.5, fr=1; for(let i=0;i<3;i++){ v+=amp*vnoise(x*fr,y*fr,s+i); fr*=2; amp*=0.5; } return v; }

  const field = React.useMemo(() => {
    const f = new Float32Array(GX * GY);
    // Squall line + bow echo, sized for mini view
    const systems = [
      { type: 'line', x1: 60, y1: 110, x2: 200, y2: 50, thick: 10, peak: 0.95 },
      { type: 'line', x1: 200, y1: 50, x2: 320, y2: 20, thick: 8, peak: 0.85 },
      { type: 'cell', x: 200, y: 70, rx: 22, ry: 14, peak: 0.95 },
      { type: 'cell', x: 240, y: 55, rx: 16, ry: 12, peak: 0.88 },
      { type: 'strat', x: 80, y: 105, rx: 70, ry: 30, peak: 0.4 },
      { type: 'strat', x: 280, y: 110, rx: 50, ry: 25, peak: 0.35 },
    ];
    for (let j = 0; j < GY; j++) {
      for (let i = 0; i < GX; i++) {
        const px = i * dx, py = j * dy;
        let v = 0;
        for (const s of systems) {
          if (s.type === 'line') {
            const vx = s.x2-s.x1, vy = s.y2-s.y1;
            const len2 = vx*vx+vy*vy;
            const tt = Math.max(0, Math.min(1, ((px-s.x1)*vx+(py-s.y1)*vy)/len2));
            const ex = s.x1+tt*vx, ey = s.y1+tt*vy;
            const d = Math.hypot(px-ex, py-ey);
            const fall = Math.max(0, 1 - d/s.thick);
            v = Math.max(v, s.peak * Math.pow(fall, 1.2));
          } else if (s.type === 'cell') {
            const d2 = ((px-s.x)/s.rx)**2 + ((py-s.y)/s.ry)**2;
            v = Math.max(v, s.peak * Math.max(0, 1-d2)**0.8);
          } else {
            const d2 = ((px-s.x)/s.rx)**2 + ((py-s.y)/s.ry)**2;
            v = Math.max(v, s.peak * Math.max(0, 1-d2));
          }
        }
        const n = fbm(px/22, py/22, 7);
        v *= 0.75 + 0.55 * n;
        const n2 = fbm(px/8, py/8, 13);
        v += (n2 - 0.5) * 0.2 * (v > 0.04 ? 1 : 0);
        f[j*GX+i] = Math.max(0, Math.min(1, v));
      }
    }
    return f;
  }, []);

  const bands = [
    [0.10, '#5a9bff'], [0.25, '#3b6dff'], [0.42, '#7a3bff'],
    [0.60, '#c73cd6'], [0.78, '#ffd74a'], [0.90, '#ff4040'],
  ];
  const cellEls = [];
  for (let j = 0; j < GY; j++) {
    for (let i = 0; i < GX; i++) {
      const v = field[j*GX+i];
      if (v < bands[0][0]) continue;
      let color = bands[0][1];
      for (let b = bands.length-1; b >= 0; b--) { if (v >= bands[b][0]) { color = bands[b][1]; break; } }
      cellEls.push(<rect key={`${i}-${j}`} x={i*dx} y={j*dy} width={dx+0.6} height={dy+0.6} fill={color} opacity={0.7 + v*0.3} />);
    }
  }

  return (
    <svg width="100%" height="100%" viewBox="0 0 360 140" preserveAspectRatio="xMidYMid slice"
         style={{ position: 'absolute', inset: 0 }}>
      <defs>
        <filter id="miniBlur" x="-10%" y="-10%" width="120%" height="120%"><feGaussianBlur stdDeviation="2.4"/></filter>
      </defs>
      {/* light map base, Apple-Weather style */}
      <rect width="360" height="140" fill="#e6efe8" />
      {/* lake */}
      <path d="M 240 -20 Q 220 30 250 70 Q 290 90 330 60 Q 350 20 320 -20 Z" fill="#c6dbe8" />
      {/* state lines */}
      <g stroke="rgba(11,18,32,0.18)" strokeWidth="0.5" fill="none">
        <path d="M 0 70 L 360 70"/>
        <path d="M 140 0 L 140 70"/>
        <path d="M 220 70 L 220 140"/>
      </g>
      {/* highways */}
      <g stroke="rgba(11,18,32,0.18)" strokeWidth="0.8" fill="none" strokeLinecap="round">
        <path d="M 0 95 Q 100 80 180 90 T 360 70"/>
        <path d="M 140 0 Q 160 60 180 140"/>
      </g>
      {/* radar field */}
      <g filter="url(#miniBlur)" style={{ mixBlendMode: 'multiply' }}>{cellEls}</g>
      {/* user location */}
      <g transform="translate(120, 80)">
        <circle r="14" fill="#8B7CFF" opacity="0.18"/>
        <circle r="6" fill="#fff" stroke="#8B7CFF" strokeWidth="2"/>
      </g>
    </svg>
  );
}

// ───── micro visualizations
function UVBar({ value }) {
  const pct = Math.min(1, value / 11);
  const color = value < 3 ? '#4ADE80' : value < 6 ? '#FFC14D' : value < 8 ? '#FF9F2E' : value < 11 ? '#FF4D6D' : '#B24BFF';
  return (
    <svg width="52" height="20" viewBox="0 0 52 20">
      <defs>
        <linearGradient id="uvg" x1="0" x2="1">
          <stop offset="0" stopColor="#4ADE80" />
          <stop offset="0.3" stopColor="#FFC14D" />
          <stop offset="0.6" stopColor="#FF9F2E" />
          <stop offset="0.85" stopColor="#FF4D6D" />
          <stop offset="1" stopColor="#B24BFF" />
        </linearGradient>
      </defs>
      <rect x="0" y="14" width="52" height="3" rx="1.5" fill="url(#uvg)" opacity="0.7" />
      <circle cx={pct * 52} cy="15.5" r="4" fill={color} stroke="#fff" strokeWidth="1.5" />
    </svg>
  );
}

function WindDial({ dir = 45 }) {
  return (
    <svg width="34" height="34" viewBox="0 0 34 34">
      <circle cx="17" cy="17" r="14" fill="none" stroke="rgba(255,255,255,0.15)" strokeWidth="1" />
      <g transform={`rotate(${dir} 17 17)`}>
        <path d="M 17 5 L 21 16 L 17 13 L 13 16 Z" fill="#8B7CFF" />
      </g>
      <text x="17" y="20" textAnchor="middle" fontSize="7" fill="rgba(255,255,255,0.5)" fontFamily="SF Mono">N</text>
    </svg>
  );
}

function FillRing({ value, color }) {
  const R = 14, C = 2 * Math.PI * R;
  return (
    <svg width="34" height="34" viewBox="0 0 34 34">
      <circle cx="17" cy="17" r={R} fill="none" stroke="rgba(255,255,255,0.12)" strokeWidth="3" />
      <circle cx="17" cy="17" r={R} fill="none" stroke={color} strokeWidth="3" strokeLinecap="round"
              strokeDasharray={`${C * value} ${C}`} transform="rotate(-90 17 17)" />
    </svg>
  );
}

function VisBars({ value }) {
  const bars = 5;
  const filled = Math.round((value / 10) * bars);
  return (
    <svg width="36" height="20" viewBox="0 0 36 20">
      {Array.from({ length: bars }).map((_, i) => (
        <rect key={i} x={i * 7} y={20 - (i + 1) * 3.6} width="5" height={(i + 1) * 3.6}
              rx="1" fill={i < filled ? '#4FB8FF' : 'rgba(255,255,255,0.15)'} />
      ))}
    </svg>
  );
}

function PressureGauge({ value }) {
  // 960-1060 range
  const pct = Math.max(0, Math.min(1, (value - 980) / 60));
  const angle = -90 + pct * 180;
  return (
    <svg width="40" height="24" viewBox="0 0 40 24">
      <path d="M 4 22 A 16 16 0 0 1 36 22" fill="none" stroke="rgba(255,255,255,0.15)" strokeWidth="3" strokeLinecap="round" />
      <path d="M 4 22 A 16 16 0 0 1 36 22" fill="none" stroke="#8B7CFF" strokeWidth="3" strokeLinecap="round"
            strokeDasharray="50.3" strokeDashoffset={50.3 - pct * 50.3} />
      <line x1="20" y1="22" x2={20 + 14 * Math.cos((angle * Math.PI) / 180)} y2={22 + 14 * Math.sin((angle * Math.PI) / 180)}
            stroke="#fff" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="20" cy="22" r="2" fill="#fff" />
    </svg>
  );
}

function AQIBar({ value }) {
  const pct = Math.min(1, value / 300);
  const color = value < 50 ? '#4ADE80' : value < 100 ? '#FFC14D' : value < 150 ? '#FF9F2E' : '#FF4D6D';
  return (
    <svg width="50" height="20" viewBox="0 0 50 20">
      <defs>
        <linearGradient id="aqg" x1="0" x2="1">
          <stop offset="0" stopColor="#4ADE80" />
          <stop offset="0.33" stopColor="#FFC14D" />
          <stop offset="0.66" stopColor="#FF9F2E" />
          <stop offset="1" stopColor="#FF4D6D" />
        </linearGradient>
      </defs>
      <rect x="0" y="14" width="50" height="3" rx="1.5" fill="url(#aqg)" opacity="0.7" />
      <circle cx={pct * 50} cy="15.5" r="4" fill={color} stroke="#fff" strokeWidth="1.5" />
    </svg>
  );
}

function SunArc({ sunrise, sunset, progress }) {
  const t = window.CUMULUS.tokens, f = window.CUMULUS.fonts;
  const W = 280, H = 90;
  // parametric arc
  const angle = Math.PI * progress;
  const cx = W / 2 - W / 2 * Math.cos(angle);
  const cy = H - 10 - (H - 20) * Math.sin(angle);
  const trailPath = `M 0 ${H - 10} A ${W / 2} ${H - 20} 0 0 1 ${W} ${H - 10}`;
  return (
    <svg width="100%" viewBox={`0 0 ${W} ${H + 20}`}>
      <defs>
        <linearGradient id="arcg" x1="0" x2="1">
          <stop offset="0" stopColor="#FFC14D" stopOpacity="0.2" />
          <stop offset="0.5" stopColor="#FFC14D" stopOpacity="0.9" />
          <stop offset="1" stopColor="#FF6E3A" stopOpacity="0.2" />
        </linearGradient>
      </defs>
      <line x1="0" y1={H - 10} x2={W} y2={H - 10} stroke="rgba(255,255,255,0.15)" strokeWidth="1" strokeDasharray="2 3" />
      <path d={trailPath} fill="none" stroke="url(#arcg)" strokeWidth="2" />
      <circle cx={cx} cy={cy} r="8" fill="#FFC14D" />
      <circle cx={cx} cy={cy} r="14" fill="#FFC14D" opacity="0.25" />
      <text x="6" y={H + 12} fontSize="10" fill={t.inkDim} fontFamily={f.mono}>{sunrise}</text>
      <text x={W - 6} y={H + 12} fontSize="10" fill={t.inkDim} fontFamily={f.mono} textAnchor="end">{sunset}</text>
      <text x={W / 2} y="14" fontSize="10" fill={t.inkMuted} fontFamily={f.mono} textAnchor="middle" letterSpacing="0.1em">DAYLIGHT</text>
    </svg>
  );
}

// ───── icons (nav + ui)
function LocationDot() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12">
      <circle cx="6" cy="6" r="5" fill="#8B7CFF" opacity="0.25" />
      <circle cx="6" cy="6" r="2.5" fill="#8B7CFF" />
    </svg>
  );
}
function IconBtn({ children }) {
  return (
    <div style={{
      width: 36, height: 36, borderRadius: 12,
      background: 'rgba(255,255,255,0.08)', border: '1px solid rgba(255,255,255,0.10)',
      display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer',
    }}>{children}</div>
  );
}
function SearchIcon() {
  return <svg width="16" height="16" viewBox="0 0 16 16" fill="none"><circle cx="7" cy="7" r="5" stroke="#fff" strokeWidth="1.6"/><path d="M11 11 L14 14" stroke="#fff" strokeWidth="1.6" strokeLinecap="round"/></svg>;
}
function MenuIcon() {
  return <svg width="16" height="16" viewBox="0 0 16 16"><circle cx="3" cy="8" r="1.6" fill="#fff"/><circle cx="8" cy="8" r="1.6" fill="#fff"/><circle cx="13" cy="8" r="1.6" fill="#fff"/></svg>;
}
function Chevron() {
  return <svg width="16" height="16" viewBox="0 0 16 16" fill="none"><path d="M6 4 L10 8 L6 12" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/></svg>;
}

Object.assign(window, { HomeScreen });

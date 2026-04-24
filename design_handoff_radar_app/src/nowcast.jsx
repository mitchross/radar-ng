// Cumulus — Nowcast screen
// 60-minute minute-by-minute precipitation with intensity bars + confidence band.

function NowcastScreen({ onBack, data }) {
  const t = window.CUMULUS.tokens;
  const f = window.CUMULUS.fonts;

  // 60 minutes of data — intensity 0..1 with confidence band
  const minutes = data.minutes; // [{i, intensity, confLo, confHi}]
  const rainStart = minutes.findIndex(m => m.intensity > 0.1);
  const peakMin = minutes.reduce((best, m, i) => m.intensity > minutes[best].intensity ? i : best, 0);
  const rainEnd = [...minutes].map(m => m.intensity).reverse().findIndex(v => v > 0.05);
  const rainEndMin = rainEnd >= 0 ? 59 - rainEnd : -1;

  return (
    <div style={{ width: '100%', height: '100%', overflow: 'auto', fontFamily: f.ui, color: t.ink, paddingBottom: 100, paddingTop: 56 }}>
      {/* header */}
      <div style={{ display: 'flex', alignItems: 'center', padding: '8px 16px 4px' }}>
        <div onClick={onBack} style={{ width: 36, height: 36, borderRadius: 12, background: 'rgba(255,255,255,0.08)', border: '1px solid rgba(255,255,255,0.10)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer' }}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none"><path d="M10 4 L6 8 L10 12" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/></svg>
        </div>
        <div style={{ flex: 1, textAlign: 'center' }}>
          <div style={{ fontSize: 11, fontFamily: f.mono, color: t.inkMuted, letterSpacing: '0.12em', fontWeight: 600 }}>HYPER-LOCAL NOWCAST</div>
          <div style={{ fontSize: 14, fontWeight: 600, marginTop: 1 }}>{data.location}</div>
        </div>
        <div style={{ width: 36 }} />
      </div>

      {/* Hero headline */}
      <div style={{ padding: '20px 20px 0' }}>
        <div style={{ fontFamily: f.display, fontWeight: 300, fontSize: 40, letterSpacing: '-0.03em', lineHeight: 1.05 }}>
          {rainStart < 0 ? 'No rain expected' :
           rainStart === 0 ? 'Raining now.' :
           <>Rain starts in<br/><span style={{ fontWeight: 500, color: t.rain }}>{rainStart} {rainStart === 1 ? 'minute' : 'minutes'}</span></>}
        </div>
        {rainStart >= 0 && rainEndMin > 0 && (
          <div style={{ fontSize: 13, color: t.inkDim, marginTop: 10 }}>
            Expected to last ~{rainEndMin - rainStart} min · peaks at <span style={{ color: t.ink }}>+{peakMin}m</span>
          </div>
        )}
      </div>

      {/* Big chart */}
      <div style={{ margin: '24px 16px 0', background: t.card, border: `1px solid ${t.cardLine}`, borderRadius: 20, padding: '16px 14px 12px' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, fontFamily: f.mono, color: t.inkMuted, letterSpacing: '0.1em', marginBottom: 10 }}>
          <span>INTENSITY · IN/HR</span>
          <span>NEXT 60 MIN</span>
        </div>
        <NowcastChart minutes={minutes} />
        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, fontFamily: f.mono, color: t.inkMuted, marginTop: 8 }}>
          <span>NOW</span><span>+15</span><span>+30</span><span>+45</span><span>+60</span>
        </div>

        {/* intensity scale */}
        <div style={{ marginTop: 14, display: 'flex', alignItems: 'center', gap: 8, fontSize: 10, fontFamily: f.mono, color: t.inkMuted }}>
          <span>LIGHT</span>
          <div style={{ flex: 1, height: 5, borderRadius: 3, background: 'linear-gradient(90deg, #7ae5a8, #4FB8FF, #1E7FFF, #8B7CFF, #FF4D6D)' }} />
          <span>INTENSE</span>
        </div>
      </div>

      {/* Key moments */}
      <SectionHeader title="KEY MOMENTS" />
      <div style={{ margin: '0 16px', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
        <KeyCard label="STARTS" value={rainStart < 0 ? '—' : `+${rainStart}m`} icon="rain" color={t.rain} />
        <KeyCard label="PEAK" value={`+${peakMin}m`} sub={`${(minutes[peakMin].intensity * 0.8).toFixed(2)}"/hr`} icon="heavyRain" color={t.hot} />
        <KeyCard label="ENDS" value={rainEndMin < 0 ? '—' : `+${rainEndMin}m`} icon="partlyCloudy" color={t.sun} />
        <KeyCard label="TOTAL" value={`${data.total}"`} sub="next hour" icon="cloudy" color={t.accent} />
      </div>

      {/* Confidence + source */}
      <SectionHeader title="FORECAST MODEL" />
      <div style={{ margin: '0 16px', background: t.card, border: `1px solid ${t.cardLine}`, borderRadius: 18, padding: 14 }}>
        <Row label="Model" value="HRRR · MRMS blend" />
        <Row label="Confidence" value={<ConfidenceBar value={data.confidence} />} />
        <Row label="Resolution" value="250m · 1 min" />
        <Row label="Last update" value={data.updatedAgo} last />
      </div>

      {/* Comparison to your block */}
      <SectionHeader title="HYPER-LOCAL VARIATION" />
      <div style={{ margin: '0 16px 20px', background: t.card, border: `1px solid ${t.cardLine}`, borderRadius: 18, padding: '14px 16px' }}>
        <div style={{ fontSize: 12, color: t.inkDim, marginBottom: 10 }}>
          Rain totals expected within 2 miles of you
        </div>
        {[
          { label: 'Your block', v: data.total, hi: true },
          { label: '½ mi north', v: data.total * 1.4 },
          { label: '½ mi south', v: data.total * 0.3 },
          { label: '1 mi east', v: data.total * 0.9 },
          { label: '1 mi west', v: data.total * 1.7 },
        ].map((r, i) => (
          <div key={i} style={{ display: 'grid', gridTemplateColumns: '90px 1fr 48px', alignItems: 'center', gap: 10, padding: '6px 0' }}>
            <div style={{ fontSize: 12, color: r.hi ? t.ink : t.inkDim, fontWeight: r.hi ? 600 : 400 }}>{r.label}</div>
            <div style={{ height: 6, borderRadius: 3, background: 'rgba(255,255,255,0.08)', overflow: 'hidden' }}>
              <div style={{ height: '100%', width: `${Math.min(100, (r.v / 2) * 100)}%`, background: r.hi ? t.accent : t.rain, borderRadius: 3 }} />
            </div>
            <div style={{ fontSize: 12, fontFamily: f.mono, textAlign: 'right', color: r.hi ? t.ink : t.inkDim }}>{r.v.toFixed(2)}"</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function NowcastChart({ minutes }) {
  const W = 320, H = 140;
  const maxI = 1;
  const barW = W / 60;
  // confidence polygon
  const topPts = minutes.map((m, i) => `${i * barW + barW/2},${H - m.confHi * H * 0.95}`).join(' ');
  const botPts = minutes.slice().reverse().map((m, i) => {
    const idx = 59 - i;
    return `${idx * barW + barW/2},${H - m.confLo * H * 0.95}`;
  }).join(' ');
  return (
    <svg width="100%" height={H + 10} viewBox={`0 0 ${W} ${H + 10}`}>
      <defs>
        <linearGradient id="rainGrad" x1="0" y1="1" x2="0" y2="0">
          <stop offset="0" stopColor="#4FB8FF" />
          <stop offset="0.5" stopColor="#1E7FFF" />
          <stop offset="1" stopColor="#8B7CFF" />
        </linearGradient>
      </defs>
      {/* y grid */}
      {[0.25, 0.5, 0.75].map(y => (
        <line key={y} x1="0" x2={W} y1={H - y * H * 0.95} y2={H - y * H * 0.95}
              stroke="rgba(255,255,255,0.06)" strokeDasharray="2 3" />
      ))}
      {/* confidence band */}
      <polygon points={`${topPts} ${botPts}`} fill="#4FB8FF" opacity="0.12" />
      {/* bars */}
      {minutes.map((m, i) => (
        <rect key={i} x={i * barW + 0.6} y={H - m.intensity * H * 0.95}
              width={barW - 1.2} height={Math.max(1, m.intensity * H * 0.95)}
              rx="1" fill="url(#rainGrad)" opacity={m.intensity > 0.02 ? 1 : 0.25} />
      ))}
      {/* baseline */}
      <line x1="0" x2={W} y1={H} y2={H} stroke="rgba(255,255,255,0.2)" />
    </svg>
  );
}

function KeyCard({ label, value, sub, icon, color }) {
  const t = window.CUMULUS.tokens, f = window.CUMULUS.fonts;
  return (
    <div style={{ background: t.card, border: `1px solid ${t.cardLine}`, borderRadius: 16, padding: '12px 14px', display: 'flex', alignItems: 'center', gap: 12 }}>
      <div style={{ width: 40, height: 40, borderRadius: 12, background: `${color}22`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <WeatherIcon kind={icon} size={28} />
      </div>
      <div style={{ flex: 1 }}>
        <div style={{ fontSize: 10, fontFamily: f.mono, color: t.inkMuted, letterSpacing: '0.1em' }}>{label}</div>
        <div style={{ fontSize: 18, fontWeight: 600, letterSpacing: '-0.01em', marginTop: 1 }}>{value}</div>
        {sub && <div style={{ fontSize: 10, color: t.inkDim, fontFamily: f.mono, marginTop: 1 }}>{sub}</div>}
      </div>
    </div>
  );
}

function Row({ label, value, last }) {
  const t = window.CUMULUS.tokens;
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '10px 0', borderBottom: last ? 'none' : `1px solid ${t.cardLine}` }}>
      <span style={{ fontSize: 12, color: t.inkDim }}>{label}</span>
      <span style={{ fontSize: 12, fontWeight: 500 }}>{value}</span>
    </div>
  );
}

function ConfidenceBar({ value }) {
  const color = value > 0.7 ? '#4ADE80' : value > 0.4 ? '#FFC14D' : '#FF9F2E';
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
      <div style={{ width: 70, height: 4, borderRadius: 2, background: 'rgba(255,255,255,0.1)' }}>
        <div style={{ width: `${value * 100}%`, height: '100%', background: color, borderRadius: 2 }} />
      </div>
      <span style={{ fontSize: 11, fontFamily: window.CUMULUS.fonts.mono, color }}>{Math.round(value * 100)}%</span>
    </div>
  );
}

Object.assign(window, { NowcastScreen });
